//! The R1 commands lane S0r pre-declared (`compare`, `suggest`, `match`, `explain`, `challenge`): hidden from
//! `--help`, and each answers `not_built_yet` (exit 2, `enth.error/1` with a hint) whatever follows it. The lane that
//! builds a command (SKR, CL) removes its row here and gives it real tests.

mod common;

use common::{round_trip, run};
use enth_cli::schema::ErrorOut;

const STUBS: [&str; 5] = ["compare", "suggest", "match", "explain", "challenge"];

#[test]
fn every_r1_stub_answers_not_built_yet() {
    for cmd in STUBS {
        for args in [vec![cmd], vec![cmd, "a.patch.json", "--level", "3", "-x"]] {
            let o = run(&args);
            let e: ErrorOut = round_trip(&o);
            assert_eq!(
                (o.code, e.exit, e.error.as_str()),
                (2, 2, "not_built_yet"),
                "{args:?}: {}",
                o.stdout
            );
            assert!(
                e.message.contains(&format!("`enth {cmd}`")),
                "{}",
                e.message
            );
            assert_ne!(e.hint, "");
        }
    }
}

#[test]
fn the_r1_stubs_stay_out_of_help() {
    let help = run(&["--help"]).stdout;
    assert!(help.contains("render"), "{help}");
    for cmd in STUBS {
        assert!(
            !help.contains(&format!("  {cmd} ")),
            "{cmd} is listed in --help:\n{help}"
        );
    }
}
