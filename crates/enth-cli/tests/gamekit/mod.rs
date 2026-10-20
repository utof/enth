//! Helpers for the game commands' tests (`suggest`, `match`): challenge codes, regenerated rounds, and a check that a
//! hint carries no number. Lane CL's file; `common` stays lane S0's.
#![allow(
    dead_code,
    clippy::disallowed_methods,
    reason = "shared by several test binaries, each using a subset; tests do file I/O"
)]

use enth_challenge::spec::PRESETS;
use enth_challenge::{ChallengeCode, PreparedRound, prepare_round};
use enth_patch::ParamAddr;
use serde_json::Value;

use crate::common::{self, Scratch};

/// The short code of Recreate preset `level` (1 to 5) at generator 1 with `seed`.
pub fn code(level: usize, seed: u64) -> String {
    let spec = PRESETS[level - 1].spec(1, seed);
    ChallengeCode::short(spec)
        .expect("a preset spec has a short code")
        .encode()
}

/// The round a code names, regenerated as the CLI does.
pub fn round(code: &str) -> PreparedRound {
    let code = ChallengeCode::decode(code).unwrap();
    prepare_round(code.spec()).unwrap()
}

/// The target's value of every drawn parameter, as the CLI would print it (`1.888kHz`, `triangle`).
pub fn target_values(round: &PreparedRound) -> Vec<(ParamAddr, String)> {
    round
        .params()
        .iter()
        .filter_map(|addr| {
            let spec = round.target().spec(addr)?;
            let v = round.target().value(addr)?;
            Some((addr.clone(), spec.display_rounded(v, 4).to_string()))
        })
        .collect()
}

/// Every JSON number in `v` with the key it sits under.
pub fn numbers(v: &Value) -> Vec<String> {
    fn go(key: &str, v: &Value, out: &mut Vec<String>) {
        match v {
            Value::Number(_) => out.push(key.to_owned()),
            Value::Array(a) => a.iter().for_each(|x| go(key, x, out)),
            Value::Object(m) => m.iter().for_each(|(k, x)| go(k, x, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    go("", v, &mut out);
    out
}

/// [`common::record`] masked, with the numbers at the JSON pointers in `keep` left as they are (the `level` of a hint
/// and the `budget` of a search are the request, not a measurement) and the values at the pointers in `hide` replaced
/// by `"<hidden>"` (what a search found moves with the DSP).
pub fn record(scratch: &Scratch, o: &enth_cli::Output, keep: &[&str], hide: &[&str]) -> String {
    let Ok(v) = serde_json::from_str::<Value>(&o.stdout) else {
        return common::record(scratch, o, true);
    };
    let mut masked = common::mask(&v);
    for pointer in keep {
        if let (Some(m), Some(orig)) = (masked.pointer_mut(pointer), v.pointer(pointer)) {
            *m = orig.clone();
        }
    }
    for pointer in hide {
        if let Some(m) = masked.pointer_mut(pointer) {
            *m = Value::from("<hidden>");
        }
    }
    scratch.clean(&format!(
        "exit: {}\n--- stdout\n{}\n--- stderr\n{}",
        o.code,
        common::pretty(&masked),
        o.stderr
    ))
}
