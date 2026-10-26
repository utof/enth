//! **Owner: lane RC** (Result codes, freeze point F1e; the F1e judge fixes the layout). Pre-declared by lane S0e (R4/R5,
//! freeze point F0e). The reference is docs/duels.md ("The two codes") and docs/challenge.md, "Version 15: the kinds
//! behind the escape".
//!
//! The **answer code** (kind [`KIND_ANSWER`](crate::result::KIND_ANSWER)): the salt, the challenge and the patch as `ops_between(start, answer)`,
//! in binary, then a 40-bit tail and code v1's GF(32) symbol ([`crate::result`] has the frame). It spoils the answer,
//! so a duel swaps it only after both players have played, and the scorecard sent first carries its salted
//! [`Commitment`].
//!
//! # Layout 1 (`result-params-v1`)
//!
//! **Widened in place on 2026-10-26** (owner decision D3, lane ANS): layout 1 had never been in a friend build, so its
//! target, parameter, place and kind fields grew to make room for S2a's catalogue (up to 62 modules, 63 parameters
//! per owner, 16 modules per section, 64 kinds) and stayed layout 1. [The guards](#widths) keep the next catalogue or
//! patch-model change from outgrowing them silently. A code made before the widening does not read on this build.
//!
//! ```text
//! header                     14   'F', kind 2, layout 1
//! salt                       64   64 bits of OS randomness, drawn by the shell once per duel (core crates have no
//!                                 RNG); it keeps the commitment from being brute-forced over a small answer space.
//!                                 Sent in the clear, so never derived from a clock or a seed (below)
//! challenge          61 or 165+   as in the scorecard (code v1's short or long payload behind a flag)
//! ops                             until END, below
//! padding                         zero bits to the end of a symbol
//! tail                       40   the first 40 bits of SHA-256("enth-answer-v1" | salt | target | answer)
//! GF(32)                      5   code v1's check symbol
//! ```
//!
//! Each op, in the order `ops_between` gives them (so every patch between the start and the answer is valid):
//!
//! | Bits | Op | Then |
//! | --- | --- | --- |
//! | `0` | a run of `SetParam`s on one target | target 6, then (parameter 6, value) pairs, then parameter 63 |
//! | `1 0000` | END | — |
//! | `1 0001` | `SetEnabled` | module 6, enabled 1 |
//! | `1 0010` | `InsertRoute` | index 6, source, destination (target 6, parameter 6), depth (an `f32`) |
//! | `1 0011` | `RemoveRoute` | route index 6 |
//! | `1 0100` | `SetDepth` | route index 6, depth |
//! | `1 0101` | `AddModule` | kind 6, index in its section 4, enabled 1, id number 6, then (parameter 6, value) pairs for the values that differ from the table's defaults, then 63 |
//! | `1 0110` | `RemoveModule` | module 6 |
//! | `1 0111` | `Move` | module 6, index 4 |
//! | `1 1000` | `SetAmpEnv` | module 6 |
//! | `1 1001` | `SetMacro` | macro 3, value (an `f32`) |
//! | `1 1010`–`1 1111` | reserved | refused as `answer_too_new` |
//!
//! - **Targets** are places in the patch as it is at that op, never ids: 0 is `voice`, 1 `master`, 2 + i the i-th
//!   module in file order (sections in order), so 6 bits name 62 modules.
//! - **An added module's id** (should-fix S1): the engine seeds a module's random streams from its id
//!   (`fnv1a64(id)`), and `osc`'s default phase mode is `free`, so a renumbered module renders other phases. An id the
//!   app gave (`<prefix><n>`, `n` from 1 to 63) keeps its number in 6 bits; **an id the player typed (`wobble`,
//!   `sam`) is never written**: it is 0, as is the lowest free `<prefix><n>` (what `add` gives), and decoding gives the
//!   lowest free id. So no typed name survives, and the canonical answer renders what the player heard. (Measured at
//!   F1e: a renumbered oscillator changed the audio, max |a − b| 0.05–1.9, and no `sound/1` or `structure/1` score in
//!   600 rows; the number is insurance against a phase-sensitive `sound/2`.)
//! - **A source** is 3 bits: 0 a module (then a target), 1 velocity, 2 key, 3 the mod wheel, 4 a macro (then 3 bits);
//!   5–7 are refused as `answer_too_new`.
//! - **Parameters** are indices into the frozen, append-only [`result-params-v1`](RESULT_PARAMS_V1) table, which fixes
//!   each parameter's key, value coding and default, so a catalogue change never moves an old code: an `f32` in the
//!   float coding (16 bits for a value with three significant digits, else the exact 33), an integer in its range's
//!   bits, an enum index in a field with room for one more item, a toggle in one bit. An index this build's table
//!   lacks is `answer_too_new`, as is a module kind it lacks.
//! - **Places:** a module added to a section already at its cap, or placed at or past it, is what a build with longer
//!   sections writes (a higher `MAX_FX`), so a decoder says `answer_too_new` ("update enth"), not `answer_mismatch`
//!   ("damaged"). A place within the cap but past the section's length is no build's, and is `answer_mismatch`.
//! - **Stripped before encoding:** `SetMeta` (`PatchMeta::author` can hold a real name), labels, macro names and
//!   automation slots, so no name, label, `meta` or slot survives; none of them changes the sound.
//! - **The salt** (the F1e review's M6, a privacy rule no test here can catch): 64 bits of **OS randomness** (std's
//!   `RandomState` keys, or `getrandom`), drawn **once per duel** by the shell and kept with the result in the duel
//!   history, and **never derived from a clock or a seed**. The code carries the salt in the clear: a salt made from
//!   the shell's `fresh_seed` (a bijection of wall-clock nanoseconds, hidden only by a 40-bit mask) would put the
//!   absolute time, to the nanosecond, into every answer code.
//!
//! **The canonical answer** is the round's start with the encoded ops applied: the player's patch without the stripped
//! fields. The tail and the commitment hash it, with the salt and the target's fingerprint:
//!
//! ```text
//! answer bytes  = fingerprint::patch_preimage(answer, phrase)      every value off its default, enabled flags,
//!                                                                  kinds, places, the amp envelope, routes, macros
//!               | "routes" | count u16 | each route's "<src>-><dst>" (u16 length, UTF-8), in summation order
//! tail          = first 40 bits of SHA-256("enth-answer-v1" | salt u64 | target fingerprint | answer bytes)
//! commitment    = first 60 bits of SHA-256("enth-commit-v1" | salt u64 | target fingerprint | answer bytes)
//! ```
//!
//! **Decoding rebuilds the answer:** it regenerates the target from the challenge, applies the ops to the start and
//! checks the tail, so an [`AnswerCode`] that decodes is an answer this build rebuilt exactly. A damaged code, or a
//! build that regenerates another start or target, is refused (`answer_mismatch`); a long challenge's fingerprint
//! prefix is checked first (`target_mismatch`). No deflated whole-patch form: an added module carries only the values
//! that differ from the table's defaults, so the op form is never much longer than a whole patch; a whole-patch form
//! would be a new layout.
//!
//! # Widths
//!
//! | Field | Bits | Names | Must cover |
//! | --- | --- | --- | --- |
//! | target | 6 | `voice`, `master` and 62 modules | 2 + the section caps (`MAX_GENERATORS` + `MAX_VOICE_FX` + `MAX_FX` + `MAX_MODULATORS`) |
//! | parameter | 6 | 63 parameters per owner (indices 0–62); 63 ends a run | every [`RESULT_PARAMS_V1`] row and every catalogue kind, reserved ones too: at most 63 parameters |
//! | place | 4 | 16 modules per section | each section cap |
//! | kind | 6 | 64 kinds, by discriminant | [`ModuleKind::ALL`], the hidden reserved kinds counted |
//! | route | 6 | 64 routes | `MAX_ROUTES` |
//! | id number | 6 | `<prefix>1`–`<prefix>63` | the module cap |
//!
//! These relations are compile-time asserts below the constants, and tests name the kind or section that breaks them.
//! A catalogue or model change that outgrows a width fails the build here, not on a friend's machine; the fix is a new
//! layout, never an edit of layout 1 (which is how the widening was allowed: layout 1 had no friend build yet).

use core::fmt;

use enth_patch::addr::{MacroIndex, ModSource, ModuleId, ParamAddr, Position, RouteAddr, Target};
use enth_patch::catalogue::{Builtin, ModuleKind};
use enth_patch::model::{MACROS, MAX_FX, MAX_GENERATORS, MAX_MODULATORS, MAX_ROUTES, MAX_VOICE_FX};
use enth_patch::{ModuleInstance, Op, Patch, Route, Section, ops_between};
use sha2::{Digest, Sha256};

use crate::code::{ChallengeCode, CodeError, EngineVersion};
use crate::fingerprint::{TargetFingerprint, patch_preimage};
use crate::model::PhraseId;
use crate::random::{self, GenError};
use crate::result::{
    Bits, Commitment, HEADER_BITS, KIND_ANSWER, LAYOUT_V1, Reader, ResultError, put_challenge,
    put_f32, take_challenge, take_f32, text, unframe,
};

/// Salt bits.
pub const SALT_BITS: u32 = 64;
/// Tail bits.
pub const TAIL_BITS: u32 = 40;
/// Tail symbols.
const TAIL_SYMBOLS: usize = 8;
/// The tail's preimage magic (frozen with layout 1).
const TAIL_MAGIC: &[u8] = b"enth-answer-v1";
/// The commitment's preimage magic (frozen with layout 1).
const COMMIT_MAGIC: &[u8] = b"enth-commit-v1";

/// Bits of a target (0 voice, 1 master, 2 + i a module): 62 modules.
const TARGET_BITS: u32 = 6;
/// Bits of a parameter index (0–62 per owner); [`RUN_END`] ends a run.
const PARAM_BITS: u32 = 6;
/// The parameter index that ends a run of values (the last one [`PARAM_BITS`] holds).
const RUN_END: u64 = 63;
/// Bits of an op code after its `1`.
const OP_BITS: u32 = 4;
/// Bits of a route index.
const ROUTE_BITS: u32 = 6;
/// Bits of an index within a section: 16 modules.
const PLACE_BITS: u32 = 4;
/// Bits of a module kind (the discriminant): 64 kinds.
const KIND_BITS: u32 = 6;
/// Bits of a macro index.
const MACRO_BITS: u32 = 3;
/// Bits of an added module's id number (`n` of `<prefix><n>`; 0 is the lowest free id).
const ID_BITS: u32 = 6;
/// Bits of a source tag.
const SOURCE_BITS: u32 = 3;

const OP_END: u64 = 0;
const OP_SET_ENABLED: u64 = 1;
const OP_INSERT_ROUTE: u64 = 2;
const OP_REMOVE_ROUTE: u64 = 3;
const OP_SET_DEPTH: u64 = 4;
const OP_ADD_MODULE: u64 = 5;
const OP_REMOVE_MODULE: u64 = 6;
const OP_MOVE: u64 = 7;
const OP_SET_AMP_ENV: u64 = 8;
const OP_SET_MACRO: u64 = 9;

/// The most modules a patch holds: the section caps summed (what the target and id-number fields must name).
const MODULE_CAP: usize = MAX_GENERATORS + MAX_VOICE_FX + MAX_FX + MAX_MODULATORS;

/// Whether `kinds` kinds (the hidden reserved ones counted) fit [`KIND_BITS`].
const fn kinds_fit(kinds: usize) -> bool {
    kinds <= 1 << KIND_BITS
}

/// Whether a section of up to `modules` modules fits [`PLACE_BITS`] (its places are 0 to `modules - 1`).
const fn places_fit(modules: usize) -> bool {
    modules <= 1 << PLACE_BITS
}

/// Whether `voice`, `master` and `modules` modules fit [`TARGET_BITS`] (targets 0, 1 and 2 + i).
const fn targets_fit(modules: usize) -> bool {
    2 + modules <= 1 << TARGET_BITS
}

/// Whether `routes` routes fit [`ROUTE_BITS`] (route indices 0 to `routes - 1`).
const fn routes_fit(routes: usize) -> bool {
    routes <= 1 << ROUTE_BITS
}

/// Whether an owner with `count` parameters fits [`PARAM_BITS`]: its indices are 0 to `count - 1`, and [`RUN_END`] ends
/// a run, so the last index must stay below it (63 parameters fit, 64 do not).
const fn params_fit(count: usize) -> bool {
    count <= RUN_END as usize
}

/// Whether `modules` modules can all hold an id number from 1 to `2^ID_BITS - 1` (0 is the lowest free id).
const fn ids_fit(modules: usize) -> bool {
    modules < 1 << ID_BITS
}

// The widths above against the patch model and the catalogue (D3, lane ANS; docs/phase-s2.md, "The answer code"). A
// change that outgrows one fails the build of this crate, so it is found when the catalogue or the model changes
// and not on a friend's machine; the answer code is the codec that cannot grow in place after a friend build. The tests
// check each relation's boundary and name the section, kind or row that breaks it. Fix by a new layout, never by
// loosening an assert.
const _: () = {
    assert!(
        kinds_fit(ModuleKind::ALL.len()),
        "KIND_BITS cannot name every ModuleKind (the hidden reserved kinds count)"
    );
    assert!(
        places_fit(MAX_GENERATORS)
            && places_fit(MAX_VOICE_FX)
            && places_fit(MAX_FX)
            && places_fit(MAX_MODULATORS),
        "PLACE_BITS cannot index a full section"
    );
    assert!(
        targets_fit(MODULE_CAP),
        "TARGET_BITS cannot name voice, master and every module of a full patch"
    );
    assert!(
        routes_fit(MAX_ROUTES),
        "ROUTE_BITS cannot index every route"
    );
    assert!(
        MACROS <= 1 << MACRO_BITS,
        "MACRO_BITS cannot name every macro"
    );
    assert!(
        ids_fit(MODULE_CAP),
        "ID_BITS cannot number every module (numbers run from 1; 0 is the lowest free id)"
    );
    assert!(
        RUN_END == (1 << PARAM_BITS) - 1,
        "RUN_END is the last value PARAM_BITS holds"
    );
    let mut i = 0;
    while i < RESULT_PARAMS_V1.len() {
        assert!(
            params_fit(RESULT_PARAMS_V1[i].1.len()),
            "a RESULT_PARAMS_V1 row has more parameters than PARAM_BITS holds (63)"
        );
        i += 1;
    }
    let mut k = 0;
    while k < ModuleKind::ALL.len() {
        assert!(
            params_fit(ModuleKind::ALL[k].params().len()),
            "a catalogue kind (a reserved one too) has more parameters than PARAM_BITS holds (63)"
        );
        k += 1;
    }
    let mut b = 0;
    while b < Builtin::ALL.len() {
        assert!(
            params_fit(Builtin::ALL[b].params().len()),
            "voice or master has more parameters than PARAM_BITS holds (63)"
        );
        b += 1;
    }
};

// ---------------------------------------------------------------------------------------------------------------
// result-params-v1
// ---------------------------------------------------------------------------------------------------------------

/// How a parameter's value is written.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Coding {
    /// An `f32`, exactly, in the float coding (16 or 33 bits).
    F32,
    /// An integer: `value − min` in `bits` bits.
    Int {
        /// The lowest value.
        min: i32,
        /// The field's width.
        bits: u8,
    },
    /// An enum item's index in `bits` bits (room for at least one more item than the catalogue had at F1e).
    Enum {
        /// The field's width.
        bits: u8,
    },
    /// Off or on, one bit.
    Toggle,
}

/// One entry of the parameter table: the key, the coding and the default an added module starts from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResultParam {
    /// The catalogue key.
    pub key: &'static str,
    /// How its value is written.
    pub coding: Coding,
    /// The default an `AddModule` starts from (the catalogue's at F1e, frozen here).
    pub default: f32,
}

const fn f(key: &'static str, default: f32) -> ResultParam {
    ResultParam {
        key,
        coding: Coding::F32,
        default,
    }
}

const fn int(key: &'static str, min: i32, bits: u8, default: f32) -> ResultParam {
    ResultParam {
        key,
        coding: Coding::Int { min, bits },
        default,
    }
}

const fn item(key: &'static str, bits: u8, default: f32) -> ResultParam {
    ResultParam {
        key,
        coding: Coding::Enum { bits },
        default,
    }
}

const fn toggle(key: &'static str, default: f32) -> ResultParam {
    ResultParam {
        key,
        coding: Coding::Toggle,
        default,
    }
}

/// `voice`'s parameters.
const VOICE_V1: &[ResultParam] = &[
    item("mode", 2, 0.0),
    int("polyphony", 1, 4, 8.0),
    f("glide", 0.0),
    item("glide_mode", 2, 0.0),
    f("pan", 0.0),
];

/// `master`'s parameters.
const MASTER_V1: &[ResultParam] = &[f("level", 0.0)];

/// `osc`.
const OSC_V1: &[ResultParam] = &[
    item("wave", 3, 2.0),
    f("semi", 0.0),
    f("fine", 0.0),
    f("level", 100.0),
    f("pw", 50.0),
    toggle("sync", 0.0),
    item("table", 3, 0.0),
    f("position", 0.0),
    item("phase_mode", 2, 0.0),
    f("phase", 0.0),
    int("seed", 0, 16, 0.0),
];

/// `noise`.
const NOISE_V1: &[ResultParam] = &[item("color", 2, 0.0), f("level", 100.0)];

/// `svf`.
const SVF_V1: &[ResultParam] = &[
    item("mode", 3, 0.0),
    f("cutoff", 2000.0),
    f("resonance", 10.0),
    f("keytrack", 0.0),
    f("drive", 0.0),
];

/// `adsr`.
const ADSR_V1: &[ResultParam] = &[
    f("attack", 5.0),
    f("decay", 300.0),
    f("sustain", 70.0),
    f("release", 400.0),
    f("velocity", 0.0),
];

/// `lfo`.
const LFO_V1: &[ResultParam] = &[
    item("shape", 3, 0.0),
    f("rate", 2.0),
    item("polarity", 2, 0.0),
    item("scope", 2, 0.0),
    item("phase_mode", 2, 1.0),
    f("phase", 0.0),
    int("seed", 0, 16, 0.0),
];

/// `reverb`.
const REVERB_V1: &[ResultParam] = &[
    f("dry", 0.0),
    f("wet", -6.0),
    f("room", 80.0),
    f("decay", 3000.0),
    f("early", 150.0),
    f("detune", 2.0),
    f("low_cut", 80.0),
    f("high_cut", 12000.0),
    f("low_damp", 1.5),
    f("high_damp", 2.5),
];

/// `comp`.
const COMP_V1: &[ResultParam] = &[
    f("threshold", -18.0),
    f("ratio", 4.0),
    f("attack", 10.0),
    f("release", 200.0),
    f("knee", 6.0),
    f("makeup", 0.0),
    f("mix", 100.0),
];

/// **`result-params-v1`**: the frozen parameter table of layout 1, one list per owner. Module kinds by their
/// discriminant (`osc`, `noise`, `svf`, `adsr`, `lfo`, `reverb`, `comp`), then `voice` and `master`. Append-only: a
/// new parameter or kind is appended (a kind past the 63rd parameter, index 62, needs a new layout: 63 ends a run),
/// and a released entry never changes. A test pins the table's bytes and checks it against the live catalogue.
pub const RESULT_PARAMS_V1: [(&str, &[ResultParam]); 9] = [
    ("osc", OSC_V1),
    ("noise", NOISE_V1),
    ("svf", SVF_V1),
    ("adsr", ADSR_V1),
    ("lfo", LFO_V1),
    ("reverb", REVERB_V1),
    ("comp", COMP_V1),
    ("voice", VOICE_V1),
    ("master", MASTER_V1),
];

/// Module kinds layout 1 knows, by discriminant.
const KINDS_V1: usize = 7;

/// Whose parameters a target has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Owner {
    Voice,
    Master,
    Module(ModuleKind),
}

impl Owner {
    /// Its row of the table (`None` for a module kind layout 1 does not know).
    fn table(self) -> Option<&'static [ResultParam]> {
        match self {
            Owner::Voice => Some(VOICE_V1),
            Owner::Master => Some(MASTER_V1),
            Owner::Module(kind) => RESULT_PARAMS_V1
                .get(kind as usize)
                .filter(|_| (kind as usize) < KINDS_V1)
                .map(|(_, row)| *row),
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The answer code
// ---------------------------------------------------------------------------------------------------------------

/// The **answer code** (kind 2): a challenge and a player's patch as a diff from its start. Build one with
/// [`AnswerCode::new`] or read one with [`AnswerCode::decode`]; either way it holds the canonical answer this build
/// rebuilt ([`AnswerCode::answer`]). Two answer codes are equal when their symbols are.
#[derive(Clone, Debug)]
pub struct AnswerCode {
    challenge: ChallengeCode,
    salt: u64,
    answer: Patch,
    target: TargetFingerprint,
    phrase: PhraseId,
    ops: usize,
    data: Vec<u8>,
}

impl PartialEq for AnswerCode {
    fn eq(&self, other: &Self) -> bool {
        self.data == other.data
    }
}

impl Eq for AnswerCode {}

impl AnswerCode {
    /// The answer code of `answer`, a patch played on `challenge`'s round, with `salt`. The patch is diffed from the
    /// challenge's start, stripped of `meta`, labels, macro names and slots, and rebuilt from the bits written, which
    /// must give the stripped patch back, module ids aside (should-fix S10). Refused (`unencodable`) when the patch is
    /// not reachable from the start by ops, or does not rebuild (a value outside its field); the challenge's own
    /// refusals (`code_too_new`, `target_mismatch`) pass through.
    ///
    /// **The salt** (the F1e review's M6): 64 bits of OS randomness (std's `RandomState` keys, or `getrandom`), drawn
    /// once per duel and kept with the result, so the same answer code can be made again to match the scorecard's
    /// commitment. **Never derive it from a clock or a seed:** the code carries it in the clear, so a salt from the
    /// shell's `fresh_seed` (wall-clock nanoseconds through a bijection) would put the absolute time into the code.
    pub fn new(
        challenge: ChallengeCode,
        salt: u64,
        answer: &Patch,
    ) -> Result<AnswerCode, ResultError> {
        let (start, target, phrase) = regenerate(&challenge)?;
        let ops = ops_between(&start, answer).ok_or_else(|| ResultError::Unencodable {
            reason: "the answer is not reachable from the round's start by ops".to_owned(),
        })?;
        let ops: Vec<Op> = ops.into_iter().filter(|op| !stripped(op)).collect();
        let mut w = Bits::header(KIND_ANSWER, LAYOUT_V1);
        w.put(salt, SALT_BITS);
        put_challenge(&mut w, &challenge);
        let ops_at = w.len();
        let count = encode_ops(&mut w, &start, &ops)?;
        let mut data = w.into_symbols();
        // Rebuild from the bits just written, so the encoder and the decoder can never disagree on the answer.
        let mut r = Reader::at(&data, ops_at);
        let (rebuilt, decoded) =
            decode_ops(&mut r, &start).map_err(|e| ResultError::Unencodable {
                reason: format!("the encoding does not rebuild: {e}"),
            })?;
        r.finish()?;
        debug_assert_eq!(decoded, count);
        if !same_ids_aside(&rebuilt, answer) {
            return Err(ResultError::Unencodable {
                reason: "the encoding does not rebuild the answer (a value outside its field)"
                    .to_owned(),
            });
        }
        let tail = digest(TAIL_MAGIC, salt, &target, &rebuilt, phrase);
        let mut w = Bits::continuing(core::mem::take(&mut data));
        w.put(top_bits(&tail, TAIL_BITS), TAIL_BITS);
        Ok(AnswerCode {
            challenge,
            salt,
            answer: rebuilt,
            target,
            phrase,
            ops: count,
            data: w.into_symbols(),
        })
    }

    /// Read an answer code and rebuild its answer. Checks, in order: the frame (symbols, length bounds, code v1's
    /// symbol, version, form, kind, layout), the challenge (as code v1 checks it), the target (a long challenge's
    /// prefix: `target_mismatch`), each op (`answer_too_new` for an op, module kind, parameter or source this build
    /// lacks, and for a module added to a section already at its cap or placed at or past it, which only a build with
    /// longer sections writes; `bad_field` for a field no encoder writes; `answer_mismatch` for an op that does not
    /// apply), the padding, then the tail (`answer_mismatch`). Never panics.
    pub fn decode(text: &str) -> Result<AnswerCode, ResultError> {
        let f = unframe(text, KIND_ANSWER, LAYOUT_V1)?;
        let data = f.data;
        if data.len() < 3 + TAIL_SYMBOLS {
            return Err(ResultError::BadLength {
                symbols: data.len() + 1,
            });
        }
        let (body, tail) = data.split_at(data.len() - TAIL_SYMBOLS);
        let tail = Reader::at(tail, 0).take(TAIL_BITS as usize)?;
        let mut r = Reader::at(body, HEADER_BITS);
        let salt = r.take(SALT_BITS as usize)?;
        let challenge = take_challenge(&mut r)?;
        let (start, target, phrase) = regenerate(&challenge)?;
        let (answer, ops) = decode_ops(&mut r, &start)?;
        r.finish()?;
        if top_bits(
            &digest(TAIL_MAGIC, salt, &target, &answer, phrase),
            TAIL_BITS,
        ) != tail
        {
            return Err(ResultError::AnswerMismatch);
        }
        Ok(AnswerCode {
            challenge,
            salt,
            answer,
            target,
            phrase,
            ops,
            data,
        })
    }

    /// The canonical text, in groups of four.
    pub fn encode(&self) -> String {
        text(self.data.clone())
    }

    /// The challenge.
    pub fn challenge(&self) -> &ChallengeCode {
        &self.challenge
    }

    /// The salt.
    pub fn salt(&self) -> u64 {
        self.salt
    }

    /// The canonical answer: the challenge's start with the encoded ops applied (the player's patch without `meta`,
    /// labels, macro names and slots, added modules numbered). What [`crate::verify`] scores.
    pub fn answer(&self) -> &Patch {
        &self.answer
    }

    /// The fingerprint of the target this build regenerated from the challenge.
    pub fn target(&self) -> TargetFingerprint {
        self.target
    }

    /// The phrase the round plays.
    pub fn phrase(&self) -> PhraseId {
        self.phrase
    }

    /// The ops encoded.
    pub fn ops(&self) -> usize {
        self.ops
    }

    /// Symbols in the code, code v1's check symbol included and hyphens not counted.
    pub fn symbols(&self) -> usize {
        self.data.len() + 1
    }

    /// The salted commitment a scorecard carries: the first 60 bits of SHA-256 over the salt, the target and the
    /// canonical answer ([module docs](self)).
    pub fn commitment(&self) -> Commitment {
        let d = digest(
            COMMIT_MAGIC,
            self.salt,
            &self.target,
            &self.answer,
            self.phrase,
        );
        Commitment::from_bits(top_bits(&d, Commitment::BITS))
            .unwrap_or_else(|| unreachable!("60 bits always fit a commitment"))
    }
}

impl fmt::Display for AnswerCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.encode())
    }
}

/// The round a challenge names: the start, the target's fingerprint and the phrase, regenerated by this build (a
/// long challenge's prefix checked: `target_mismatch`).
fn regenerate(
    challenge: &ChallengeCode,
) -> Result<(Patch, TargetFingerprint, PhraseId), ResultError> {
    let spec = challenge.spec();
    let gen_err = |e: GenError| match e {
        GenError::Spec(e) => ResultError::Challenge(CodeError::Spec(e)),
        other => ResultError::Unencodable {
            reason: other.to_string(),
        },
    };
    let start = random::start(spec.template).map_err(gen_err)?;
    let generated = random::generate(spec, &start).map_err(gen_err)?;
    challenge.verify(&generated.fingerprint, EngineVersion::CURRENT)?;
    Ok((start, generated.fingerprint, generated.phrase))
}

/// Whether an op is stripped before encoding: it never changes the sound, and it can carry a name.
fn stripped(op: &Op) -> bool {
    matches!(
        op,
        Op::SetMeta { .. } | Op::SetLabel { .. } | Op::SetMacroName { .. } | Op::SetSlot { .. }
    )
}

/// The canonical answer's bytes ([module docs](self)).
pub fn answer_bytes(answer: &Patch, phrase: PhraseId) -> Vec<u8> {
    let mut out = patch_preimage(answer, phrase);
    out.extend_from_slice(b"routes");
    out.extend_from_slice(&(answer.routes.len() as u16).to_be_bytes());
    for r in &answer.routes {
        let key = format!("{}->{}", r.src, r.dst);
        out.extend_from_slice(&(key.len() as u16).to_be_bytes());
        out.extend_from_slice(key.as_bytes());
    }
    out
}

/// SHA-256 of `magic | salt | target | answer bytes`.
fn digest(
    magic: &[u8],
    salt: u64,
    target: &TargetFingerprint,
    answer: &Patch,
    phrase: PhraseId,
) -> [u8; 32] {
    Sha256::new()
        .chain_update(magic)
        .chain_update(salt.to_be_bytes())
        .chain_update(target.0)
        .chain_update(answer_bytes(answer, phrase))
        .finalize()
        .into()
}

/// The first `bits` (at most 64) bits of a digest.
fn top_bits(d: &[u8; 32], bits: u32) -> u64 {
    let top = u64::from_be_bytes([d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7]]);
    top >> (64 - bits)
}

impl Bits {
    /// A writer that continues after whole symbols.
    fn continuing(syms: Vec<u8>) -> Bits {
        let mut w = Bits::default();
        for s in syms {
            w.put(u64::from(s), 5);
        }
        w
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Ops to bits
// ---------------------------------------------------------------------------------------------------------------

fn unencodable(reason: impl Into<String>) -> ResultError {
    ResultError::Unencodable {
        reason: reason.into(),
    }
}

/// A module's place in file order.
fn ordinal(p: &Patch, id: &ModuleId) -> Option<usize> {
    p.modules().position(|(_, m)| &m.id == id)
}

/// A target's number: 0 voice, 1 master, 2 + the module's place.
fn target_number(p: &Patch, target: &Target) -> Result<u64, ResultError> {
    match target {
        Target::Voice => Ok(0),
        Target::Master => Ok(1),
        Target::Module(id) => ordinal(p, id)
            .map(|i| 2 + i as u64)
            .ok_or_else(|| unencodable(format!("no module {id}"))),
    }
}

/// Whose parameters a target has.
fn owner_of(p: &Patch, target: &Target) -> Result<Owner, ResultError> {
    match target {
        Target::Voice => Ok(Owner::Voice),
        Target::Master => Ok(Owner::Master),
        Target::Module(id) => p
            .find(id)
            .map(|(_, m)| Owner::Module(m.kind))
            .ok_or_else(|| unencodable(format!("no module {id}"))),
    }
}

/// A parameter's index in its owner's row.
fn param_index(owner: Owner, key: &str) -> Result<(u64, Coding), ResultError> {
    owner
        .table()
        .and_then(|row| {
            row.iter()
                .position(|e| e.key == key)
                .map(|i| (i as u64, row[i].coding))
        })
        .ok_or_else(|| unencodable(format!("{key} is not in result-params-v1")))
}

fn put_module(w: &mut Bits, p: &Patch, id: &ModuleId) -> Result<(), ResultError> {
    w.put(target_number(p, &Target::Module(id.clone()))?, TARGET_BITS);
    Ok(())
}

fn put_value(w: &mut Bits, coding: Coding, v: f32) -> Result<(), ResultError> {
    match coding {
        Coding::F32 => put_f32(w, v),
        Coding::Int { min, bits } => {
            let x = v as i64 - i64::from(min);
            if f64::from(v).fract() != 0.0 || x < 0 || x >> bits != 0 {
                return Err(unencodable(format!("{v} does not fit its field")));
            }
            w.put(x as u64, u32::from(bits));
        }
        Coding::Enum { bits } => {
            if f64::from(v).fract() != 0.0 || v < 0.0 || (v as u64) >> bits != 0 {
                return Err(unencodable(format!("item {v} does not fit its field")));
            }
            w.put(v as u64, u32::from(bits));
        }
        Coding::Toggle => w.put_bool(v != 0.0),
    }
    Ok(())
}

fn put_source(w: &mut Bits, p: &Patch, src: &ModSource) -> Result<(), ResultError> {
    match src {
        ModSource::Module(id) => {
            w.put(0, SOURCE_BITS);
            put_module(w, p, id)?;
        }
        ModSource::Velocity => w.put(1, SOURCE_BITS),
        ModSource::Key => w.put(2, SOURCE_BITS),
        ModSource::ModWheel => w.put(3, SOURCE_BITS),
        ModSource::Macro(i) => {
            w.put(4, SOURCE_BITS);
            w.put(i.get() as u64, MACRO_BITS);
        }
    }
    Ok(())
}

fn route_index(p: &Patch, route: &RouteAddr) -> Result<u64, ResultError> {
    p.routes
        .iter()
        .position(|r| r.src == route.src && r.dst == route.dst)
        .map(|i| i as u64)
        .ok_or_else(|| unencodable(format!("no route {route}")))
}

fn put_op_code(w: &mut Bits, code: u64) {
    w.put(1, 1);
    w.put(code, OP_BITS);
}

/// Write `ops` (already stripped) over `start`, then END; returns the number of ops written. The ops are applied to a
/// copy of the start as they are written, so each target is numbered against the patch as it is at that op.
fn encode_ops(w: &mut Bits, start: &Patch, ops: &[Op]) -> Result<usize, ResultError> {
    let mut p = start.clone();
    // The open run's target number.
    let mut run: Option<u64> = None;
    for op in ops {
        match op {
            Op::SetParam { addr, value } => {
                let number = target_number(&p, addr.target())?;
                if run != Some(number) {
                    if run.is_some() {
                        w.put(RUN_END, PARAM_BITS);
                    }
                    w.put(0, 1);
                    w.put(number, TARGET_BITS);
                    run = Some(number);
                }
                let (index, coding) = param_index(owner_of(&p, addr.target())?, addr.param())?;
                w.put(index, PARAM_BITS);
                put_value(w, coding, *value)?;
            }
            other => {
                if run.take().is_some() {
                    w.put(RUN_END, PARAM_BITS);
                }
                encode_op(w, &p, other)?;
            }
        }
        enth_patch::apply(&mut p, op).map_err(|e| unencodable(e.to_string()))?;
    }
    if run.is_some() {
        w.put(RUN_END, PARAM_BITS);
    }
    put_op_code(w, OP_END);
    Ok(ops.len())
}

/// Write one op other than `SetParam` against the patch `p` it applies to.
fn encode_op(w: &mut Bits, p: &Patch, op: &Op) -> Result<(), ResultError> {
    match op {
        Op::SetEnabled { id, enabled } => {
            put_op_code(w, OP_SET_ENABLED);
            put_module(w, p, id)?;
            w.put_bool(*enabled);
        }
        Op::InsertRoute { index, route } => {
            put_op_code(w, OP_INSERT_ROUTE);
            w.put(*index as u64, ROUTE_BITS);
            put_source(w, p, &route.src)?;
            let target = route.dst.target();
            w.put(target_number(p, target)?, TARGET_BITS);
            let (param, _) = param_index(owner_of(p, target)?, route.dst.param())?;
            w.put(param, PARAM_BITS);
            put_f32(w, route.depth);
        }
        Op::RemoveRoute { route } => {
            put_op_code(w, OP_REMOVE_ROUTE);
            w.put(route_index(p, route)?, ROUTE_BITS);
        }
        Op::SetDepth { route, depth } => {
            put_op_code(w, OP_SET_DEPTH);
            w.put(route_index(p, route)?, ROUTE_BITS);
            put_f32(w, *depth);
        }
        Op::AddModule { at, module } => {
            put_op_code(w, OP_ADD_MODULE);
            let row = Owner::Module(module.kind).table().ok_or_else(|| {
                unencodable(format!("module kind {} is not in layout 1", module.kind))
            })?;
            w.put(module.kind as u64, KIND_BITS);
            w.put(at.index as u64, PLACE_BITS);
            w.put_bool(module.enabled);
            w.put(id_number(p, module), ID_BITS);
            for (i, e) in row.iter().enumerate() {
                let v = module
                    .get(e.key)
                    .ok_or_else(|| unencodable(format!("{} has no {}", module.kind, e.key)))?;
                if v.to_bits() != e.default.to_bits() {
                    w.put(i as u64, PARAM_BITS);
                    put_value(w, e.coding, v)?;
                }
            }
            w.put(RUN_END, PARAM_BITS);
        }
        Op::RemoveModule { id } => {
            put_op_code(w, OP_REMOVE_MODULE);
            put_module(w, p, id)?;
        }
        Op::Move { id, to } => {
            put_op_code(w, OP_MOVE);
            put_module(w, p, id)?;
            w.put(to.index as u64, PLACE_BITS);
        }
        Op::SetAmpEnv { id } => {
            put_op_code(w, OP_SET_AMP_ENV);
            put_module(w, p, id)?;
        }
        Op::SetMacro { index, value } => {
            put_op_code(w, OP_SET_MACRO);
            w.put(index.get() as u64, MACRO_BITS);
            put_f32(w, *value);
        }
        other => return Err(unencodable(format!("{} is not encoded", other.name()))),
    }
    Ok(())
}

/// The id number an added module is written with (should-fix S1): `n` for an id `<prefix><n>` the app gave (`n` from
/// 1 to 63), unless it is the lowest free id, which decoding gives anyway; 0 for that and for any id the player typed.
fn id_number(p: &Patch, module: &ModuleInstance) -> u64 {
    let prefix = module.kind.spec().id_prefix;
    let n = module
        .id
        .as_str()
        .strip_prefix(prefix)
        .and_then(|digits| digits.parse::<u32>().ok())
        .filter(|&n| (1..1 << ID_BITS).contains(&n))
        .filter(|&n| ModuleId::numbered(prefix, n).is_ok_and(|id| id == module.id));
    match n {
        Some(n) if p.free_id(module.kind) != module.id => u64::from(n),
        _ => 0,
    }
}

/// Whether `rebuilt` is `answer` without the stripped fields (`meta`, labels, macro names, slots; the engine stamp),
/// module ids aside (should-fix S10): the same modules (kind, enabled flag, values) in the same places, the same routes
/// between the same places with the same depths, the amplitude envelope in the same place, and the same voice, master
/// and macro values. A field that overflowed its bits would rebuild another patch, and is refused rather than shared.
fn same_ids_aside(rebuilt: &Patch, answer: &Patch) -> bool {
    let a: Vec<(Position, &ModuleInstance)> = answer.modules().collect();
    let b: Vec<(Position, &ModuleInstance)> = rebuilt.modules().collect();
    if a.len() != b.len() {
        return false;
    }
    let same_modules = a.iter().zip(&b).all(|((pa, ma), (pb, mb))| {
        pa == pb && ma.kind == mb.kind && ma.enabled == mb.enabled && ma.values == mb.values
    });
    // An id of the answer, as the rebuilt patch names the module in its place.
    let renamed = |id: &ModuleId| {
        a.iter()
            .position(|(_, m)| &m.id == id)
            .map(|i| b[i].1.id.clone())
    };
    let same_target = |x: &Target, y: &Target| match (x, y) {
        (Target::Module(i), Target::Module(j)) => renamed(i).as_ref() == Some(j),
        _ => x == y,
    };
    let same_source = |x: &ModSource, y: &ModSource| match (x, y) {
        (ModSource::Module(i), ModSource::Module(j)) => renamed(i).as_ref() == Some(j),
        _ => x == y,
    };
    let same_routes = answer.routes.len() == rebuilt.routes.len()
        && answer.routes.iter().zip(&rebuilt.routes).all(|(x, y)| {
            same_source(&x.src, &y.src)
                && same_target(x.dst.target(), y.dst.target())
                && x.dst.param() == y.dst.param()
                && x.depth == y.depth
        });
    same_modules
        && same_routes
        && renamed(&answer.voice.amp_env).as_ref() == Some(&rebuilt.voice.amp_env)
        && answer.voice.values == rebuilt.voice.values
        && answer.master == rebuilt.master
        && answer
            .macros
            .iter()
            .zip(&rebuilt.macros)
            .all(|(x, y)| x.value == y.value)
}

// ---------------------------------------------------------------------------------------------------------------
// Bits to ops
// ---------------------------------------------------------------------------------------------------------------

fn bad(field: &'static str, value: u64) -> ResultError {
    ResultError::BadField { field, value }
}

/// The target a number names in `p`.
fn take_target(r: &mut Reader<'_>, p: &Patch) -> Result<Target, ResultError> {
    match r.take(TARGET_BITS as usize)? {
        0 => Ok(Target::Voice),
        1 => Ok(Target::Master),
        n => p
            .modules()
            .nth(n as usize - 2)
            .map(|(_, m)| Target::Module(m.id.clone()))
            .ok_or(bad("target", n)),
    }
}

/// A module (not `voice` or `master`) a number names in `p`.
fn take_module(r: &mut Reader<'_>, p: &Patch) -> Result<ModuleId, ResultError> {
    match take_target(r, p)? {
        Target::Module(id) => Ok(id),
        Target::Voice => Err(bad("module", 0)),
        Target::Master => Err(bad("module", 1)),
    }
}

fn owner_in(p: &Patch, target: &Target) -> Result<Owner, ResultError> {
    match target {
        Target::Voice => Ok(Owner::Voice),
        Target::Master => Ok(Owner::Master),
        Target::Module(id) => p
            .find(id)
            .map(|(_, m)| Owner::Module(m.kind))
            .ok_or(ResultError::AnswerMismatch),
    }
}

/// The entry a parameter index names (`None` at the run's end).
fn take_param(
    r: &mut Reader<'_>,
    owner: Owner,
) -> Result<Option<&'static ResultParam>, ResultError> {
    let i = r.take(PARAM_BITS as usize)?;
    if i == RUN_END {
        return Ok(None);
    }
    let row = owner.table().ok_or(ResultError::AnswerTooNew {
        what: "module kind",
        value: 0,
    })?;
    row.get(i as usize)
        .map(Some)
        .ok_or(ResultError::AnswerTooNew {
            what: "parameter",
            value: i,
        })
}

/// A place in `section` (`PLACE_BITS` bits). What a build with longer sections writes (a higher `MAX_FX`) is not a
/// damaged code but one this build is too old to hold, so it is `answer_too_new` ("update enth"): an add to a section
/// already at its cap (`module count`, the count the add would reach), and a place at or past the cap (`module place`).
/// A place this build's section could hold but does not reach yet (past its length) is `answer_mismatch`, from `apply`.
fn take_place(
    r: &mut Reader<'_>,
    p: &Patch,
    section: Section,
    adding: bool,
) -> Result<usize, ResultError> {
    let index = r.take(PLACE_BITS as usize)?;
    let len = p.section(section).len();
    if adding && len >= section.max_len() {
        return Err(ResultError::AnswerTooNew {
            what: "module count",
            value: len as u64 + 1,
        });
    }
    if index as usize >= section.max_len() {
        return Err(ResultError::AnswerTooNew {
            what: "module place",
            value: index,
        });
    }
    Ok(index as usize)
}

fn take_value(r: &mut Reader<'_>, coding: Coding) -> Result<f32, ResultError> {
    Ok(match coding {
        Coding::F32 => take_f32(r)?,
        Coding::Int { min, bits } => (i64::from(min) + r.take(usize::from(bits))? as i64) as f32,
        Coding::Enum { bits } => r.take(usize::from(bits))? as f32,
        Coding::Toggle => f32::from(u8::from(r.bool()?)),
    })
}

fn take_source(r: &mut Reader<'_>, p: &Patch) -> Result<ModSource, ResultError> {
    match r.take(SOURCE_BITS as usize)? {
        0 => Ok(ModSource::Module(take_module(r, p)?)),
        1 => Ok(ModSource::Velocity),
        2 => Ok(ModSource::Key),
        3 => Ok(ModSource::ModWheel),
        4 => {
            let i = r.take(MACRO_BITS as usize)?;
            MacroIndex::new(i as usize)
                .map(ModSource::Macro)
                .ok_or(bad("macro", i))
        }
        tag => Err(ResultError::AnswerTooNew {
            what: "source",
            value: tag,
        }),
    }
}

fn take_route(r: &mut Reader<'_>, p: &Patch) -> Result<RouteAddr, ResultError> {
    let i = r.take(ROUTE_BITS as usize)?;
    p.routes
        .get(i as usize)
        .map(|route| RouteAddr {
            src: route.src.clone(),
            dst: route.dst.clone(),
        })
        .ok_or(bad("route", i))
}

fn addr(target: Target, key: &str) -> Result<ParamAddr, ResultError> {
    ParamAddr::new(target, key).map_err(|_| ResultError::AnswerMismatch)
}

fn apply(p: &mut Patch, op: &Op) -> Result<(), ResultError> {
    enth_patch::apply(p, op)
        .map(drop)
        .map_err(|_| ResultError::AnswerMismatch)
}

/// Read ops until END and apply them to a copy of `start`: the canonical answer and the number of ops.
fn decode_ops(r: &mut Reader<'_>, start: &Patch) -> Result<(Patch, usize), ResultError> {
    let mut p = start.clone();
    let mut count = 0usize;
    loop {
        if !r.bool()? {
            // A run of SetParams on one target.
            let target = take_target(r, &p)?;
            let owner = owner_in(&p, &target)?;
            let mut values = 0;
            while let Some(e) = take_param(r, owner)? {
                let value = take_value(r, e.coding)?;
                apply(
                    &mut p,
                    &Op::SetParam {
                        addr: addr(target.clone(), e.key)?,
                        value,
                    },
                )?;
                values += 1;
            }
            if values == 0 {
                return Err(bad("run", 0));
            }
            count += values;
            continue;
        }
        let op = match r.take(OP_BITS as usize)? {
            OP_END => return Ok((p, count)),
            OP_SET_ENABLED => Op::SetEnabled {
                id: take_module(r, &p)?,
                enabled: r.bool()?,
            },
            OP_INSERT_ROUTE => {
                let index = r.take(ROUTE_BITS as usize)? as usize;
                let src = take_source(r, &p)?;
                let target = take_target(r, &p)?;
                let owner = owner_in(&p, &target)?;
                let e = take_param(r, owner)?.ok_or(bad("parameter", RUN_END))?;
                let depth = take_f32(r)?;
                Op::InsertRoute {
                    index,
                    route: Route {
                        src,
                        dst: addr(target, e.key)?,
                        depth,
                    },
                }
            }
            OP_REMOVE_ROUTE => Op::RemoveRoute {
                route: take_route(r, &p)?,
            },
            OP_SET_DEPTH => Op::SetDepth {
                route: take_route(r, &p)?,
                depth: take_f32(r)?,
            },
            OP_ADD_MODULE => {
                let k = r.take(KIND_BITS as usize)?;
                let kind = ModuleKind::ALL
                    .get(k as usize)
                    .copied()
                    .filter(|_| (k as usize) < KINDS_V1)
                    .ok_or(ResultError::AnswerTooNew {
                        what: "module kind",
                        value: k,
                    })?;
                let index = take_place(r, &p, kind.section(), true)?;
                let enabled = r.bool()?;
                let n = r.take(ID_BITS as usize)?;
                let id = if n == 0 {
                    p.free_id(kind)
                } else {
                    ModuleId::numbered(kind.spec().id_prefix, n as u32)
                        .ok()
                        .filter(|id| p.find(id).is_none())
                        .ok_or(bad("module id", n))?
                };
                let owner = Owner::Module(kind);
                let row = owner.table().ok_or(ResultError::AnswerTooNew {
                    what: "module kind",
                    value: k,
                })?;
                let mut module = ModuleInstance::new(id, kind);
                module.enabled = enabled;
                // The table's frozen defaults first, then the values the code carries.
                for e in row {
                    let i = kind.param_index(e.key).ok_or(ResultError::AnswerMismatch)?;
                    module.values[i] = e.default;
                }
                while let Some(e) = take_param(r, owner)? {
                    let value = take_value(r, e.coding)?;
                    let i = kind.param_index(e.key).ok_or(ResultError::AnswerMismatch)?;
                    module.values[i] = value;
                }
                Op::AddModule {
                    at: Position {
                        section: kind.section(),
                        index,
                    },
                    module,
                }
            }
            OP_REMOVE_MODULE => Op::RemoveModule {
                id: take_module(r, &p)?,
            },
            OP_MOVE => {
                let id = take_module(r, &p)?;
                let section = p
                    .find(&id)
                    .map(|(at, _)| at.section)
                    .ok_or(ResultError::AnswerMismatch)?;
                Op::Move {
                    id,
                    to: Position {
                        section,
                        index: take_place(r, &p, section, false)?,
                    },
                }
            }
            OP_SET_AMP_ENV => Op::SetAmpEnv {
                id: take_module(r, &p)?,
            },
            OP_SET_MACRO => {
                let i = r.take(MACRO_BITS as usize)?;
                Op::SetMacro {
                    index: MacroIndex::new(i as usize).ok_or(bad("macro", i))?,
                    value: take_f32(r)?,
                }
            }
            code => {
                return Err(ResultError::AnswerTooNew {
                    what: "op",
                    value: code,
                });
            }
        };
        apply(&mut p, &op)?;
        count += 1;
    }
}

#[cfg(test)]
mod tests;
