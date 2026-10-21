//! The R3 command lane S0s pre-declared (`create`): hidden from `--help`, and it answers `not_built_yet` (exit 2,
//! `enth.error/1` with a hint) whatever follows it. The lane that builds it (SK3) removes this file and gives the
//! command real tests. (`stubs.rs` holds R2's row for `lesson`.)

mod common;

use common::{round_trip, run};
use enth_cli::schema::ErrorOut;

const STUBS: [&str; 1] = ["create"];

#[test]
fn every_r3_stub_answers_not_built_yet() {
    for cmd in STUBS {
        for args in [
            vec![cmd],
            vec![
                cmd,
                "--target",
                "x.wav",
                "--candidates",
                "8",
                "--seconds",
                "60",
                "-x",
            ],
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
fn the_r3_stubs_stay_out_of_help() {
    let help = run(&["--help"]).stdout;
    assert!(help.contains("render"), "{help}");
    for cmd in STUBS {
        assert!(
            !help.contains(&format!("  {cmd} ")),
            "{cmd} is listed in --help:\n{help}"
        );
    }
}
