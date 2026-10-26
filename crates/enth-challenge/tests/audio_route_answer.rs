//! An audio route between generators (lane RTE, S2a) in an answer code: the route's source tag is a module, its
//! destination an `audio` parameter. This pins what the answer code does with one today, so a later change to the
//! layout cannot silently break it: the code must round-trip when every module and parameter on the route is one
//! the layout has rows for, and must be refused with a code of its own (never mis-decoded) when it names a kind the
//! layout has not been given rows for yet. Lane RTE did not change `answer.rs`.

use enth_challenge::answer::AnswerCode;
use enth_challenge::random;
use enth_challenge::{ChallengeCode, SpecPreset};
use enth_patch::{
    ModSource, ModuleId, ModuleInstance, ModuleKind, Op, Patch, Position, Route, apply,
};

fn short(level: u8, seed: u64) -> ChallengeCode {
    ChallengeCode::short(SpecPreset::by_id(level).unwrap().spec(1, seed)).unwrap()
}

fn id(s: &str) -> ModuleId {
    ModuleId::new(s).unwrap()
}

/// The target of a round plus a `wt` and an audio route from `osc1` into its `pm`.
fn with_audio_route(c: &ChallengeCode, depth: f32) -> Patch {
    let mut p = random::target(c.spec()).unwrap().patch;
    let wt = ModuleInstance::new(id("wt1"), ModuleKind::Wt);
    let at = Position {
        section: ModuleKind::Wt.section(),
        index: p.generators.len(),
    };
    apply(&mut p, &Op::AddModule { at, module: wt }).unwrap();
    let index = p.routes.len();
    let route = Route {
        src: ModSource::Module(id("osc1")),
        dst: "wt1.pm".parse().unwrap(),
        depth,
    };
    apply(&mut p, &Op::InsertRoute { index, route }).unwrap();
    p
}

#[test]
fn an_audio_route_into_a_reserved_kind_is_encodable_or_refused_never_misread() {
    let c = short(3, 42);
    let p = with_audio_route(&c, 35.0);
    match AnswerCode::new(c, 0xABCD, &p) {
        Ok(code) => {
            // Encodable: it must round-trip to the same canonical answer, route and all.
            let back = AnswerCode::decode(&code.encode()).expect("decodes");
            assert_eq!(back, code);
            assert_eq!(back.answer().routes, code.answer().routes);
            let r = back
                .answer()
                .routes
                .iter()
                .find(|r| r.dst.to_string() == "wt1.pm")
                .expect("the audio route survived");
            assert_eq!(r.src.to_string(), "osc1");
            assert_eq!(r.depth, 35.0);
        }
        Err(e) => {
            // Refused at encode time with a code of its own: layout 1 has no parameter rows for `wt` yet (lane CAT
            // or ANS adds them), so the module cannot be added, let alone routed to. The route's source side needs
            // nothing: a generator is a module, source tag 0.
            assert_eq!(e.code(), "unencodable", "{e}");
            assert!(e.to_string().contains("wt"), "{e}");
        }
    }
}
