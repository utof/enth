//! S2a's reserved kinds (lane S0d, freeze F0m) in the CLI: `enth modules` never lists one and no suggestion names
//! one, while a reserved kind's key still resolves (`enth params wt`), so the module lanes can work with it
//! (docs/phase-s2.md, rule 6).

mod common;

use common::{round_trip, run};
use enth_cli::schema::{ModulesOut, ParamsOut};
use enth_patch::ModuleKind;

fn hidden_keys() -> Vec<&'static str> {
    ModuleKind::ALL
        .iter()
        .filter(|k| k.is_hidden())
        .map(|k| k.key())
        .collect()
}

#[test]
fn modules_never_lists_a_reserved_kind() {
    let hidden = hidden_keys();
    assert_eq!(hidden.len(), 11);
    let out: ModulesOut = round_trip(&run(&["modules"]));
    let listed: Vec<&str> = out.modules.iter().map(|m| m.kind.as_str()).collect();
    assert_eq!(
        listed,
        ["osc", "noise", "svf", "adsr", "lfo", "reverb", "comp"]
    );
    for key in hidden {
        assert!(!listed.contains(&key), "{key} is listed");
    }
}

#[test]
fn a_reserved_kind_resolves_by_key_but_is_never_suggested() {
    let wt: ParamsOut = round_trip(&run(&["params", "wt"]));
    assert_eq!(wt.kind, "wt");
    assert_eq!(wt.section.as_deref(), Some("generators"));
    let unknown = run(&["params", "wtx"]);
    assert_ne!(unknown.code, 0);
    for key in hidden_keys() {
        assert!(
            !unknown.stdout.contains(&format!("\"{key}\"")),
            "{key} is suggested or listed: {}",
            unknown.stdout
        );
    }
}
