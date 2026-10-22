//! The R4 and R5 commands lane S0e pre-declared (`ear` and `duel`): hidden from `--help`, and they answer
//! `not_built_yet` (exit 2, `enth.error/1` with a hint) whatever follows them. The lanes that build them (SKE and SKD)
//! remove their rows and give the commands real tests. (`stubs.rs` held R2's row for `lesson`.)

mod common;

use common::{round_trip, run};
use enth_cli::schema::ErrorOut;

const STUBS: [&str; 2] = ["ear", "duel"];

#[test]
fn every_r4_and_r5_stub_answers_not_built_yet() {
    for cmd in STUBS {
        for args in [
            vec![cmd],
            vec![cmd, "result", "challenge.txt", "--level", "2", "-x"],
        ] {
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
fn the_r4_and_r5_stubs_stay_out_of_help() {
    let help = run(&["--help"]).stdout;
    assert!(help.contains("render"), "{help}");
    for cmd in STUBS {
        assert!(
            !help.contains(&format!("  {cmd} ")),
            "{cmd} is listed in --help:\n{help}"
        );
    }
}
