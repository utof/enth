//! The R2 commands pre-declared as stubs: hidden from `--help`, answering `not_built_yet` (exit 2, `enth.error/1`
//! with a hint) whatever follows them. Lane S0l declared `lesson`; lane SKL built it (F2l, `tests/lesson.rs`) and
//! removed its row, so the list is empty until another R2 stub joins.

mod common;

use common::{round_trip, run};
use enth_cli::schema::ErrorOut;

const STUBS: [&str; 0] = [];

#[test]
fn every_r2_stub_answers_not_built_yet() {
    for cmd in STUBS {
        for args in [
            vec![cmd],
            vec![cmd, "show", "basics/01-saw", "--step", "3", "-x"],
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
fn the_r2_stubs_stay_out_of_help() {
    let help = run(&["--help"]).stdout;
    assert!(help.contains("render"), "{help}");
    for cmd in STUBS {
        assert!(
            !help.contains(&format!("  {cmd} ")),
            "{cmd} is listed in --help:\n{help}"
        );
    }
}
