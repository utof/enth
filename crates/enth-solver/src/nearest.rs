//! **Owner: lane TP** (R3a, freeze point F1s; MT left this as a documented stub, cut to R2 and then to R3). The nearest
//! patches to a sound: **k-NN retrieval over a pre-rendered bank of generator patches** (docs/r3.md, "The start
//! portfolio and k-NN retrieval"; report 01 §7), one of the starts of a create job
//! ([`portfolio`](crate::create::portfolio)). It keeps the stub's meaning, "the nearest preset as a start"; R2's
//! [`closest`](crate::closest) is another feature.
//!
//! # The embedding (`knn-logmel-rel/1`)
//!
//! A clip becomes [`DIMS`] bytes, computed the same way for the bank and for a query ([`embed`]):
//!
//! 1. Metric v1's features ([`Features`]): the clip scaled to −23 LUFS, natural-log mel magnitudes at the 1024-point
//!    resolution (hop 512).
//! 2. **Four time segments from the note-on:** 0–0.1, 0.1–0.3, 0.3–0.8 and 0.8–2 s. Per segment and mel band, the mean
//!    of the frames' mel magnitudes, then its log (metric v1.1's pooling, `pooled`). A segment past the end of the
//!    clip is the 1e-5 floor: a short sound reads as silence there, never as missing.
//! 3. **Bands relative to the pitch:** [`REL_BANDS`] third-octave bands from half the reference frequency up
//!    (`f_ref · 2^((k − 3) / 3)`), each read off the mel spectrum by linear interpolation on the mel axis. The
//!    reference is the measured pitch when the clip is pitched ([`reference_hz`]), else a fixed C3, so a pitched
//!    sound matches a bank patch played at another note by its harmonic shape, and noise keeps absolute bands. Bank
//!    patches are rendered at one note ([`REF_NOTE`]); this is what lets one render per patch serve every query note.
//! 4. **Bands the clip does not carry are masked:** below the mel range (30 Hz), or whose upper edge is above the
//!    mel range (16 kHz), half the sample rate, or the upload's detected bandwidth (the intake's codec low-pass).
//! 5. **Quantised to one byte** ([`quantise`]): 1/16 neper steps (about 0.54 dB) from −12 nepers, code 0 meaning
//!    "masked". A byte, not the f16 the plan's default named, halves the bank (decision recorded in docs/r3.md).
//!
//! The distance is the mean absolute difference over the bands both clips carry, in integer arithmetic on the codes,
//! so a ranking is the same on every platform for the same codes; ties go to the lower bank index. A pair sharing
//! fewer than a quarter of the bands has no distance.
//!
//! # The bank file (`ENTHBANK`, format 1)
//!
//! Generator patches are bit-exact from their spec, so the bank stores **specs, not patches**: per entry the seed,
//! the difficulty, `musical`, the measured pitch offset, the target fingerprint's 32-bit prefix (a generator that
//! drifted is caught when a neighbour is regenerated), and the embedding. The header pins the generator and template,
//! the reference performance and render spec, the embedding version, the SHA-256 of the GR3 exclusion lists the
//! builder asserted, and the SHA-256 of the entries. [`Bank::parse`] checks all of it; the builder is
//! `tools/datagen/src/bank.rs` (`cargo xtask bank`), which writes through [`Bank::to_bytes`] so the format lives here.
//!
//! Pure like the rest of the solver: no I/O (the shell passes the file's bytes), no threads, no clock; a query over
//! 50,000 entries is a few million integer operations.

use core::fmt;

use enth_analysis::Descriptors;
use enth_analysis::features::{BANDS, FFT_SIZES, FeatureCfg, Features, MEL_RANGE_HZ};
use enth_challenge::random::{self, GenError};
use enth_challenge::spec::{Category, RandomSpec};
use enth_patch::{ModuleKind, Op, ParamAddr, Patch, apply};
use sha2::{Digest, Sha256};

/// The embedding's id. A change to its definition gets a new id (and a new [`EMBEDDING_VERSION`]).
pub const EMBEDDING_ID: &str = "knn-logmel-rel/1";

/// The embedding's version, as the bank header stores it.
pub const EMBEDDING_VERSION: u16 = 1;

/// The time segments from the note-on, in seconds: attack, early body, late body, tail.
pub const SEGMENTS_S: [(f64, f64); 4] = [(0.0, 0.1), (0.1, 0.3), (0.3, 0.8), (0.8, 2.0)];

/// Third-octave bands per segment, from half the reference frequency to `2^(24/3) = 256` times it.
pub const REL_BANDS: usize = 28;

/// Bytes per embedding.
pub const DIMS: usize = SEGMENTS_S.len() * REL_BANDS;

/// The lowest band's offset in third octaves from the reference (−3: half the reference).
const LOWEST_THIRD: f64 = -3.0;

/// The mel resolution the embedding reads: index 1 of [`FFT_SIZES`] (1024 points).
const EMBED_RES: usize = 1;

/// The log-mel floor (`ln 1e-5`, metric v1's): what a segment past the end of the clip reads.
const FLOOR: f64 = -11.512_925_464_970_229;

/// Quantisation: code = round((v − `Q_LO`) · `Q_STEPS`), clamped to 1–255; 0 is a masked band.
pub const Q_LO: f64 = -12.0;

/// Codes per neper.
pub const Q_STEPS: f64 = 16.0;

/// The note bank patches are rendered at (C3, the generator's reference key).
pub const REF_NOTE: u8 = 48;

/// The reference frequency of an unpitched clip: C3, 130.81 Hz, for the bank and the query alike.
#[must_use]
pub fn ref_hz() -> f64 {
    note_hz(REF_NOTE)
}

/// A pitch reading counts as pitched when YIN found at least this share of the loud frames voiced.
const PITCHED_VOICED: f64 = 0.5;

/// A pair of embeddings needs at least this many shared bands to have a distance.
const MIN_SHARED: u32 = (DIMS / 4) as u32;

/// The frequency of a MIDI note (A4 = 69 = 440 Hz).
#[must_use]
pub fn note_hz(note: u8) -> f64 {
    440.0 * libm::pow(2.0, (f64::from(note) - 69.0) / 12.0)
}

/// The measured pitch of a clip with these descriptors, when YIN found most of the loud frames voiced.
#[must_use]
pub fn pitched_hz(desc: Option<&Descriptors>) -> Option<f64> {
    desc.and_then(|d| d.pitch.as_ref())
        .filter(|p| p.f0_hz > 0.0 && p.f0_hz.is_finite() && p.voiced >= PITCHED_VOICED)
        .map(|p| p.f0_hz)
}

/// The embedding's reference frequency for a clip with these descriptors: [`pitched_hz`], else [`ref_hz`].
#[must_use]
pub fn reference_hz(desc: Option<&Descriptors>) -> f64 {
    pitched_hz(desc).unwrap_or_else(ref_hz)
}

/// A bank entry's pitch offset: the measured pitch relative to `note`, in cents, or [`UNPITCHED`].
#[must_use]
pub fn pitch_cents(desc: Option<&Descriptors>, note: u8) -> i16 {
    pitched_hz(desc).map_or(UNPITCHED, |f| {
        let c = libm::round(1200.0 * libm::log2(f / note_hz(note)));
        c.clamp(f64::from(i16::MIN + 1), f64::from(i16::MAX)) as i16
    })
}

/// The quantised value of `v` nepers: 1–255.
#[must_use]
pub fn quantise(v: f64) -> u8 {
    let c = libm::round((v - Q_LO) * Q_STEPS);
    if c.is_nan() {
        1
    } else {
        c.clamp(1.0, 255.0) as u8
    }
}

/// The value a code stands for (`None` for a masked band).
#[must_use]
pub fn dequantise(code: u8) -> Option<f64> {
    (code != 0).then(|| Q_LO + f64::from(code) / Q_STEPS)
}

/// The mel scale of `enth_analysis::features` (HTK: `2595 · log10(1 + f / 700)`).
fn hz_to_mel(f: f64) -> f64 {
    2595.0 * libm::log10(1.0 + f / 700.0)
}

/// A clip's embedding: [`DIMS`] codes, segment-major (segment 0's bands, then segment 1's...).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Embedding {
    /// One code per segment and band; 0 is a masked band.
    pub codes: [u8; DIMS],
}

impl Embedding {
    /// An embedding with every band masked (a silent clip): it is at no distance from anything.
    pub const EMPTY: Embedding = Embedding { codes: [0; DIMS] };

    /// Whether every band is masked.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.codes.iter().all(|&c| c == 0)
    }

    /// The mean absolute difference over the bands both carry, in nepers; `None` when they share fewer than a
    /// quarter of the bands.
    #[must_use]
    pub fn distance(&self, other: &Embedding) -> Option<f64> {
        let (sum, n) = l1(&self.codes, &other.codes)?;
        Some(f64::from(sum) / (f64::from(n) * Q_STEPS))
    }
}

/// The integer L1 sum and the shared band count, or `None` below [`MIN_SHARED`].
fn l1(a: &[u8; DIMS], b: &[u8; DIMS]) -> Option<(u32, u32)> {
    let (mut sum, mut n) = (0u32, 0u32);
    for (&x, &y) in a.iter().zip(b) {
        if x != 0 && y != 0 {
            sum += u32::from(x.abs_diff(y));
            n += 1;
        }
    }
    (n >= MIN_SHARED).then_some((sum, n))
}

/// The embedding of a clip (see the [module docs](self)).
///
/// - `features`: the clip's [`Features`] from a [`FeatureExtractor`](enth_analysis::features::FeatureExtractor) with
///   `cfg` (metric v1's settings at the clip's rate; loudness-normalised);
/// - `onset_s`: the note-on, in clip time (the intake keeps a few ms of pre-roll before it);
/// - `reference_hz`: from [`reference_hz`];
/// - `top_hz`: the highest frequency the clip carries (half its rate, or the intake's detected bandwidth if lower).
#[must_use]
pub fn embed(
    features: &Features,
    cfg: &FeatureCfg,
    onset_s: f64,
    reference_hz: f64,
    top_hz: f64,
) -> Embedding {
    let mut out = Embedding::EMPTY;
    if features.silent
        || !reference_hz.is_finite()
        || reference_hz <= 0.0
        || features.logmel.len() <= EMBED_RES
    {
        return out;
    }
    let lm = &features.logmel[EMBED_RES];
    let n_fft = FFT_SIZES[EMBED_RES];
    let hop = (n_fft / cfg.hop_div.max(1)).max(1);
    let sr = cfg.sample_rate;
    let frames = lm.len() / BANDS;
    let top = top_hz.min(MEL_RANGE_HZ.1).min(0.5 * sr);
    let (mel_lo, mel_hi) = (hz_to_mel(MEL_RANGE_HZ.0), hz_to_mel(MEL_RANGE_HZ.1));
    let mut seg = [0f64; BANDS];
    for (s, &(t0, t1)) in SEGMENTS_S.iter().enumerate() {
        let mut acc = [0f64; BANDS];
        let mut n = 0usize;
        for i in 0..frames {
            let t = (i * hop + n_fft / 2) as f64 / sr - onset_s;
            if t >= t0 && t < t1 {
                for (b, a) in acc.iter_mut().enumerate() {
                    *a += libm::exp(f64::from(lm[i * BANDS + b]));
                }
                n += 1;
            }
        }
        for (b, v) in seg.iter_mut().enumerate() {
            *v = if n == 0 {
                FLOOR
            } else {
                libm::log(acc[b] / n as f64)
            };
        }
        for k in 0..REL_BANDS {
            let f = reference_hz * libm::pow(2.0, (k as f64 + LOWEST_THIRD) / 3.0);
            let upper = f * libm::pow(2.0, 1.0 / 6.0);
            if f < MEL_RANGE_HZ.0 || upper > top {
                continue;
            }
            // Band b's centre sits at mel point b + 1 of BANDS + 2 equally spaced points.
            let p = ((hz_to_mel(f) - mel_lo) / (mel_hi - mel_lo) * (BANDS + 1) as f64 - 1.0)
                .clamp(0.0, (BANDS - 1) as f64);
            let i0 = libm::floor(p) as usize;
            let i1 = (i0 + 1).min(BANDS - 1);
            let frac = p - i0 as f64;
            let v = seg[i0] * (1.0 - frac) + seg[i1] * frac;
            out.codes[s * REL_BANDS + k] = quantise(v);
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------------------------
// The bank
// ---------------------------------------------------------------------------------------------------------------

/// The file's magic bytes.
pub const BANK_MAGIC: [u8; 8] = *b"ENTHBANK";

/// The file format this build reads and writes. A newer one is refused ([`BankError::TooNew`]).
pub const BANK_FORMAT: u16 = 1;

/// Header bytes before the entries.
const HEADER_LEN: usize = 120;

/// Bytes per entry: seed 8, difficulty 1, musical 1, pitch offset 2, fingerprint prefix 4, the embedding.
pub const ENTRY_LEN: usize = 16 + DIMS;

/// The pitch offset an unpitched entry records.
pub const UNPITCHED: i16 = i16::MIN;

/// How the bank's patches were rendered and which generator made them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BankHeader {
    /// The file format ([`BANK_FORMAT`]).
    pub format: u16,
    /// The generator version of every spec (generator v1 is frozen).
    pub generator: u16,
    /// The generator template of every spec (0, the starter superset).
    pub template: u8,
    /// The embedding version ([`EMBEDDING_VERSION`]).
    pub embedding: u16,
    /// The note every patch was played at ([`REF_NOTE`]).
    pub note: u8,
    /// Its velocity, 0–1.
    pub velocity: f32,
    /// How long it was held, in seconds.
    pub hold_s: f32,
    /// The render's length, in seconds.
    pub duration_s: f32,
    /// The render's rate.
    pub sample_rate: u32,
    /// The render seed.
    pub render_seed: u64,
    /// The SHA-256 of the GR3 exclusion lists the builder asserted (`content/gr3/exclude-*.txt`, in the order synth,
    /// real, degraded, each file's bytes hashed in turn).
    pub exclusions_sha256: [u8; 32],
}

/// One bank patch: its spec (the header supplies the rest), what was measured, and its embedding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BankEntry {
    /// The generator seed (never inside a GR3 seed range: the builder asserts it).
    pub seed: u64,
    /// The spec's difficulty (a level preset: 25, 50, 75 or 100).
    pub difficulty: u8,
    /// The spec's `musical`.
    pub musical: u8,
    /// The measured pitch of the render relative to [`BankHeader::note`], in cents; [`UNPITCHED`] when none. A
    /// neighbour is transposed by it so that it plays at the query's note.
    pub pitch_cents: i16,
    /// The first four bytes of the target fingerprint, big-endian (what a challenge code's long form holds).
    pub fingerprint_prefix: u32,
    /// The embedding.
    pub embedding: Embedding,
}

/// Why a bank could not be read or a neighbour regenerated.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum BankError {
    /// Not a bank file (magic), or cut short.
    NotABank,
    /// A newer format than this build reads.
    TooNew {
        /// The file's format.
        format: u16,
    },
    /// The embedding is another version or size than this build computes.
    Embedding {
        /// The file's embedding version.
        version: u16,
        /// Its bytes per entry.
        dims: u16,
    },
    /// The file is not the size its header says.
    Length {
        /// Bytes expected.
        expected: usize,
        /// Bytes found.
        got: usize,
    },
    /// The entries do not hash to the header's SHA-256.
    Hash,
    /// A neighbour's spec does not generate (a generator or template this build lacks).
    Generate(String),
    /// A regenerated patch's fingerprint does not match the bank's: the generator drifted.
    Fingerprint {
        /// The entry.
        index: usize,
    },
}

impl BankError {
    /// A stable snake-case code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            BankError::NotABank => "not_a_bank",
            BankError::TooNew { .. } => "bank_too_new",
            BankError::Embedding { .. } => "bank_embedding",
            BankError::Length { .. } => "bank_length",
            BankError::Hash => "bank_hash",
            BankError::Generate(_) => "bank_generate",
            BankError::Fingerprint { .. } => "bank_fingerprint",
        }
    }
}

impl fmt::Display for BankError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BankError::NotABank => f.write_str("not an enth k-NN bank (bad magic or cut short)"),
            BankError::TooNew { format } => write!(
                f,
                "bank format {format} is newer than this build reads ({BANK_FORMAT})"
            ),
            BankError::Embedding { version, dims } => write!(
                f,
                "bank embedding v{version} with {dims} bytes; this build computes v{EMBEDDING_VERSION} with {DIMS}"
            ),
            BankError::Length { expected, got } => {
                write!(f, "bank is {got} bytes; its header says {expected}")
            }
            BankError::Hash => f.write_str("bank entries do not match the header's SHA-256"),
            BankError::Generate(why) => write!(f, "a bank spec does not generate: {why}"),
            BankError::Fingerprint { index } => write!(
                f,
                "bank entry {index} regenerates a different target: the generator drifted"
            ),
        }
    }
}

impl std::error::Error for BankError {}

/// One retrieved neighbour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Neighbour {
    /// Its index in the bank.
    pub index: usize,
    /// Its embedding distance to the query, in nepers.
    pub distance: f64,
}

/// A k-NN bank: the header and its entries.
#[derive(Clone, Debug, PartialEq)]
pub struct Bank {
    header: BankHeader,
    entries: Vec<BankEntry>,
}

fn put_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn put_u64(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn get<const N: usize>(b: &[u8], at: usize) -> [u8; N] {
    let mut out = [0u8; N];
    out.copy_from_slice(&b[at..at + N]);
    out
}

impl Bank {
    /// A bank from its header and entries (the builder's, and tests').
    #[must_use]
    pub fn new(header: BankHeader, entries: Vec<BankEntry>) -> Bank {
        Bank { header, entries }
    }

    /// The header.
    #[must_use]
    pub fn header(&self) -> &BankHeader {
        &self.header
    }

    /// The entries, in bank order.
    #[must_use]
    pub fn entries(&self) -> &[BankEntry] {
        &self.entries
    }

    /// The number of entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the bank has no entry.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn body(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.entries.len() * ENTRY_LEN);
        for e in &self.entries {
            put_u64(&mut out, e.seed);
            out.push(e.difficulty);
            out.push(e.musical);
            out.extend_from_slice(&e.pitch_cents.to_le_bytes());
            out.extend_from_slice(&e.fingerprint_prefix.to_be_bytes());
            out.extend_from_slice(&e.embedding.codes);
        }
        out
    }

    /// The file's bytes: the header (little-endian, [`BANK_MAGIC`] first), then the entries.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let h = &self.header;
        let body = self.body();
        let mut out = Vec::with_capacity(HEADER_LEN + body.len());
        out.extend_from_slice(&BANK_MAGIC);
        put_u16(&mut out, BANK_FORMAT);
        put_u16(&mut out, h.generator);
        out.push(h.template);
        out.push(h.note);
        put_u16(&mut out, h.embedding);
        put_u16(&mut out, DIMS as u16);
        put_u16(&mut out, 0); // reserved
        put_u32(&mut out, h.velocity.to_bits());
        put_u32(&mut out, h.hold_s.to_bits());
        put_u32(&mut out, h.duration_s.to_bits());
        put_u32(&mut out, h.sample_rate);
        put_u64(&mut out, h.render_seed);
        put_u32(&mut out, self.entries.len() as u32);
        put_u32(&mut out, 0); // reserved
        put_u32(&mut out, 0); // reserved
        out.extend_from_slice(&h.exclusions_sha256);
        out.extend_from_slice(&Sha256::digest(&body));
        debug_assert_eq!(out.len(), HEADER_LEN);
        out.extend_from_slice(&body);
        out
    }

    /// Read a bank file, checking its magic, format, embedding, length and the entries' SHA-256.
    ///
    /// # Errors
    /// See [`BankError`].
    pub fn parse(bytes: &[u8]) -> Result<Bank, BankError> {
        if bytes.len() < HEADER_LEN || bytes[..8] != BANK_MAGIC {
            return Err(BankError::NotABank);
        }
        let u16_at = |at| u16::from_le_bytes(get(bytes, at));
        let u32_at = |at| u32::from_le_bytes(get(bytes, at));
        let format = u16_at(8);
        if format > BANK_FORMAT || format == 0 {
            return Err(BankError::TooNew { format });
        }
        let (embedding, dims) = (u16_at(14), u16_at(16));
        if embedding != EMBEDDING_VERSION || usize::from(dims) != DIMS {
            return Err(BankError::Embedding {
                version: embedding,
                dims,
            });
        }
        let count = u32_at(44) as usize;
        let expected = HEADER_LEN + count * ENTRY_LEN;
        if bytes.len() != expected {
            return Err(BankError::Length {
                expected,
                got: bytes.len(),
            });
        }
        let body = &bytes[HEADER_LEN..];
        let digest: [u8; 32] = Sha256::digest(body).into();
        if digest != get::<32>(bytes, 88) {
            return Err(BankError::Hash);
        }
        let header = BankHeader {
            format,
            generator: u16_at(10),
            template: bytes[12],
            embedding,
            note: bytes[13],
            velocity: f32::from_bits(u32_at(20)),
            hold_s: f32::from_bits(u32_at(24)),
            duration_s: f32::from_bits(u32_at(28)),
            sample_rate: u32_at(32),
            render_seed: u64::from_le_bytes(get(bytes, 36)),
            exclusions_sha256: get(bytes, 56),
        };
        let entries = body
            .as_chunks::<ENTRY_LEN>()
            .0
            .iter()
            .map(|e| BankEntry {
                seed: u64::from_le_bytes(get(e, 0)),
                difficulty: e[8],
                musical: e[9],
                pitch_cents: i16::from_le_bytes(get(e, 10)),
                fingerprint_prefix: u32::from_be_bytes(get(e, 12)),
                embedding: Embedding { codes: get(e, 16) },
            })
            .collect();
        Ok(Bank { header, entries })
    }

    /// The SHA-256 of the file's bytes (what a result records to name the bank it used).
    #[must_use]
    pub fn sha256(&self) -> [u8; 32] {
        Sha256::digest(self.to_bytes()).into()
    }

    /// The generator spec of entry `index`.
    ///
    /// # Panics
    /// If `index` is out of range.
    #[must_use]
    pub fn spec(&self, index: usize) -> RandomSpec {
        let e = &self.entries[index];
        RandomSpec {
            generator: self.header.generator,
            seed: e.seed,
            difficulty: e.difficulty,
            amounts: [None; Category::COUNT],
            musical: e.musical,
            template: self.header.template,
        }
    }

    /// The `k` nearest entries to `query`, nearest first (ties to the lower index), skipping any entry closer than
    /// `min_spread` nepers to one already taken, so the starts differ. Entries sharing too few bands with the query
    /// are never returned. Deterministic: integer distances, a total order.
    #[must_use]
    pub fn nearest(&self, query: &Embedding, k: usize, min_spread: f64) -> Vec<Neighbour> {
        if k == 0 || query.is_empty() {
            return Vec::new();
        }
        let mut scored: Vec<(u32, u32, usize)> = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| l1(&query.codes, &e.embedding.codes).map(|(s, n)| (s, n, i)))
            .collect();
        // sum_a / n_a < sum_b / n_b exactly, by cross-multiplication; then the index.
        scored.sort_by(|a, b| {
            (u64::from(a.0) * u64::from(b.1))
                .cmp(&(u64::from(b.0) * u64::from(a.1)))
                .then(a.2.cmp(&b.2))
        });
        let mut out: Vec<Neighbour> = Vec::with_capacity(k);
        for (sum, n, i) in scored {
            if out.len() == k {
                break;
            }
            let e = &self.entries[i].embedding;
            let crowded = out.iter().any(|t| {
                self.entries[t.index]
                    .embedding
                    .distance(e)
                    .is_some_and(|d| d < min_spread)
            });
            if !crowded {
                out.push(Neighbour {
                    index: i,
                    distance: f64::from(sum) / (f64::from(n) * Q_STEPS),
                });
            }
        }
        out
    }

    /// Entry `index` as a start patch: its spec regenerated (checking the fingerprint prefix), then every enabled
    /// oscillator transposed by the measured pitch offset, rounded to semitones, so that the patch played at a
    /// query's note sounds at that note (the search's pitch heuristic then fixes the cents).
    ///
    /// # Errors
    /// [`BankError::Generate`] if the spec does not generate in this build, [`BankError::Fingerprint`] if it
    /// generates another target.
    ///
    /// # Panics
    /// If `index` is out of range.
    pub fn patch(&self, index: usize) -> Result<Patch, BankError> {
        let spec = self.spec(index);
        let generated =
            random::target(&spec).map_err(|e: GenError| BankError::Generate(e.to_string()))?;
        if generated.fingerprint.prefix().0 != self.entries[index].fingerprint_prefix {
            return Err(BankError::Fingerprint { index });
        }
        let mut patch = generated.patch;
        let cents = self.entries[index].pitch_cents;
        if cents != UNPITCHED {
            let semis = libm::round(f64::from(cents) / 100.0);
            if semis != 0.0 {
                transpose(&mut patch, -semis);
            }
        }
        Ok(patch)
    }
}

/// Move every enabled oscillator's `semi` by `semis`, clamped to its range (through ops).
fn transpose(patch: &mut Patch, semis: f64) {
    let addrs: Vec<ParamAddr> = patch
        .generators
        .iter()
        .filter(|m| m.enabled && m.kind == ModuleKind::Osc)
        .filter_map(|m| ParamAddr::module(m.id.clone(), "semi").ok())
        .collect();
    for addr in addrs {
        let (Some(spec), Some(v)) = (patch.spec(&addr), patch.value(&addr)) else {
            continue;
        };
        let value = spec.clamp((f64::from(v) + semis) as f32);
        // A clamped value is in range, so the op applies; a failure would leave the start as generated.
        let _ = apply(patch, Op::SetParam { addr, value });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use enth_analysis::features::FeatureExtractor;
    use enth_render::{Performance, RenderSpec, render};

    fn header() -> BankHeader {
        BankHeader {
            format: BANK_FORMAT,
            generator: 1,
            template: 0,
            embedding: EMBEDDING_VERSION,
            note: REF_NOTE,
            velocity: 0.8,
            hold_s: 1.0,
            duration_s: 2.0,
            sample_rate: 48_000,
            render_seed: 7,
            exclusions_sha256: [3; 32],
        }
    }

    fn entry(seed: u64, fill: u8) -> BankEntry {
        let mut codes = [fill; DIMS];
        codes[0] = 0;
        BankEntry {
            seed,
            difficulty: 50,
            musical: 100,
            pitch_cents: 12,
            fingerprint_prefix: 0xDEAD_BEEF,
            embedding: Embedding { codes },
        }
    }

    fn embed_patch(patch: &Patch, note: u8) -> Embedding {
        let perf = Performance::note(note, 0.8, 1.0, 2.0);
        let spec = RenderSpec::default();
        let out = render(patch, &perf, &spec).unwrap();
        let cfg = FeatureCfg {
            sample_rate: f64::from(spec.sample_rate),
            ..FeatureCfg::default()
        };
        let f = FeatureExtractor::new(cfg).extract(&out.mid());
        let mut d = enth_analysis::Describer::new(f64::from(spec.sample_rate));
        let desc = d
            .describe_some(
                &out.mid(),
                crate::eval::note_timing(&perf),
                enth_analysis::descriptors::Want::CORE,
            )
            .ok();
        embed(&f, &cfg, 0.0, reference_hz(desc.as_ref()), 24_000.0)
    }

    #[test]
    fn quantisation_round_trips_within_half_a_step() {
        for v in [-11.5, -3.25, 0.0, 1.7] {
            let back = dequantise(quantise(v)).unwrap();
            assert!((back - v).abs() <= 0.5 / Q_STEPS + 1e-12, "{v} -> {back}");
        }
        assert_eq!(quantise(-100.0), 1);
        assert_eq!(quantise(100.0), 255);
        assert_eq!(dequantise(0), None);
    }

    #[test]
    fn the_file_round_trips_and_refuses_damage() {
        let bank = Bank::new(header(), vec![entry(11, 40), entry(12, 90)]);
        let bytes = bank.to_bytes();
        assert_eq!(bytes.len(), HEADER_LEN + 2 * ENTRY_LEN);
        assert_eq!(Bank::parse(&bytes).unwrap(), bank);

        let mut bad = bytes.clone();
        *bad.last_mut().unwrap() ^= 1;
        assert_eq!(Bank::parse(&bad), Err(BankError::Hash));
        assert_eq!(
            Bank::parse(&bytes[..bytes.len() - 1]),
            Err(BankError::Length {
                expected: bytes.len(),
                got: bytes.len() - 1
            })
        );
        let mut newer = bytes.clone();
        newer[8] = 2;
        assert_eq!(Bank::parse(&newer), Err(BankError::TooNew { format: 2 }));
        assert_eq!(Bank::parse(b"not a bank at all"), Err(BankError::NotABank));
    }

    #[test]
    fn nearest_is_ordered_by_distance_then_index_and_spreads() {
        let bank = Bank::new(
            header(),
            vec![
                entry(1, 100),
                entry(2, 50),
                entry(3, 52),
                entry(4, 50),
                entry(5, 200),
            ],
        );
        let q = Embedding { codes: [51; DIMS] };
        let got: Vec<usize> = bank.nearest(&q, 3, 0.0).iter().map(|n| n.index).collect();
        // 2 and 4 tie at 1 code; 3 is 1 code away too (52 vs 51): all three tie, so index order.
        assert_eq!(got, vec![1, 2, 3]);
        // With a spread of 1 neper, entries within 16 codes of a taken one are skipped.
        let spread: Vec<usize> = bank.nearest(&q, 3, 1.0).iter().map(|n| n.index).collect();
        assert_eq!(spread, vec![1, 0, 4]);
        assert_eq!(
            bank.nearest(&Embedding::EMPTY, 3, 0.0),
            Vec::<Neighbour>::new()
        );
    }

    #[test]
    fn a_pitched_sound_matches_itself_an_octave_up_better_than_another_wave() {
        let saw = Patch::init();
        let mut sine = Patch::init();
        apply(
            &mut sine,
            Op::SetParam {
                addr: "osc1.wave".parse().unwrap(),
                value: 0.0,
            },
        )
        .unwrap();
        let saw48 = embed_patch(&saw, 48);
        let saw60 = embed_patch(&saw, 60);
        let sine60 = embed_patch(&sine, 60);
        let same = saw48.distance(&saw60).unwrap();
        let other = saw48.distance(&sine60).unwrap();
        assert!(same < other, "saw C3 vs saw C4 {same}, vs sine C4 {other}");
        assert_eq!(saw48, embed_patch(&saw, 48), "deterministic");
    }

    #[test]
    fn bands_above_the_bandwidth_are_masked() {
        let perf = Performance::note(48, 0.8, 1.0, 2.0);
        let spec = RenderSpec::default();
        let out = render(&Patch::init(), &perf, &spec).unwrap();
        let cfg = FeatureCfg::default();
        let f = FeatureExtractor::new(cfg).extract(&out.mid());
        let full = embed(&f, &cfg, 0.0, ref_hz(), 24_000.0);
        let cut = embed(&f, &cfg, 0.0, ref_hz(), 2_000.0);
        let carried = |e: &Embedding| e.codes.iter().filter(|&&c| c != 0).count();
        assert!(carried(&cut) < carried(&full));
        for k in 0..REL_BANDS {
            let f = ref_hz() * libm::pow(2.0, (k as f64 + LOWEST_THIRD) / 3.0);
            if f * libm::pow(2.0, 1.0 / 6.0) > 2_000.0 {
                assert_eq!(cut.codes[k], 0, "band {k} at {f} Hz");
            }
        }
        // A silent clip embeds to nothing.
        let silent = FeatureExtractor::new(cfg).extract(&vec![0.0; 48_000]);
        assert!(embed(&silent, &cfg, 0.0, ref_hz(), 24_000.0).is_empty());
    }

    #[test]
    fn a_neighbour_regenerates_and_a_drifted_fingerprint_is_refused() {
        let spec = RandomSpec {
            generator: 1,
            seed: 123_456,
            difficulty: 100,
            amounts: [None; Category::COUNT],
            musical: 100,
            template: 0,
        };
        let g = random::target(&spec).unwrap();
        let mut e = entry(123_456, 60);
        e.difficulty = 100;
        e.fingerprint_prefix = g.fingerprint.prefix().0;
        e.pitch_cents = UNPITCHED;
        let bank = Bank::new(header(), vec![e]);
        assert_eq!(bank.patch(0).unwrap(), g.patch);
        let mut drifted = e;
        drifted.fingerprint_prefix ^= 1;
        let bank = Bank::new(header(), vec![drifted]);
        assert_eq!(bank.patch(0), Err(BankError::Fingerprint { index: 0 }));
    }
}
