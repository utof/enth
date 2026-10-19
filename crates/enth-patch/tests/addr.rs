//! Property tests for ids and addresses: text round trips, serde round trips, and fuzz-lite (random text never
//! panics a parser and always yields a typed error with a code and a hint).

use enth_patch::addr::{
    AddrError, MacroIndex, ModSource, ModuleId, ParamAddr, Position, RouteAddr, Target,
    is_reserved_id,
};
use enth_patch::catalogue::{Builtin, CATALOGUE};
use enth_patch::model::Section;
use proptest::prelude::*;

fn module_id() -> impl Strategy<Value = ModuleId> {
    "[a-z][a-z0-9_]{0,15}".prop_filter_map("reserved", |s| ModuleId::new(&s).ok())
}

fn param_addr() -> impl Strategy<Value = ParamAddr> {
    let module_keys: Vec<&'static str> = CATALOGUE
        .iter()
        .flat_map(|m| m.params.iter().map(|p| p.key))
        .collect();
    let voice_keys: Vec<&'static str> = Builtin::Voice.params().iter().map(|p| p.key).collect();
    let master_keys: Vec<&'static str> = Builtin::Master.params().iter().map(|p| p.key).collect();
    prop_oneof![
        (module_id(), prop::sample::select(module_keys))
            .prop_map(|(id, k)| ParamAddr::module(id, k).unwrap()),
        prop::sample::select(voice_keys).prop_map(|k| ParamAddr::new(Target::Voice, k).unwrap()),
        prop::sample::select(master_keys).prop_map(|k| ParamAddr::new(Target::Master, k).unwrap()),
    ]
}

fn mod_source() -> impl Strategy<Value = ModSource> {
    prop_oneof![
        module_id().prop_map(ModSource::Module),
        Just(ModSource::Velocity),
        Just(ModSource::Key),
        Just(ModSource::ModWheel),
        (0usize..8).prop_map(|i| ModSource::Macro(MacroIndex::new(i).unwrap())),
    ]
}

fn position() -> impl Strategy<Value = Position> {
    prop::sample::select(Section::ALL.to_vec()).prop_flat_map(|section| {
        (0..section.max_len()).prop_map(move |index| Position { section, index })
    })
}

fn assert_typed(e: &AddrError) {
    assert_ne!(e.code(), "");
    assert_ne!(e.hint(), "");
    assert_ne!(e.to_string(), "");
}

proptest! {
    #[test]
    fn ids_round_trip(id in module_id()) {
        prop_assert_eq!(id.to_string().parse::<ModuleId>(), Ok(id.clone()));
        prop_assert!(!is_reserved_id(id.as_str()));
    }

    #[test]
    fn addresses_round_trip(a in param_addr()) {
        prop_assert_eq!(a.to_string().parse::<ParamAddr>(), Ok(a.clone()));
        let json = serde_json::to_string(&a).unwrap();
        prop_assert_eq!(serde_json::from_str::<ParamAddr>(&json).unwrap(), a);
    }

    #[test]
    fn sources_and_routes_round_trip(src in mod_source(), dst in param_addr()) {
        prop_assert_eq!(src.to_string().parse::<ModSource>(), Ok(src.clone()));
        let r = RouteAddr { src, dst };
        prop_assert_eq!(r.to_string().parse::<RouteAddr>(), Ok(r.clone()));
        let json = serde_json::to_string(&r).unwrap();
        prop_assert_eq!(serde_json::from_str::<RouteAddr>(&json).unwrap(), r);
    }

    #[test]
    fn positions_round_trip(p in position()) {
        prop_assert_eq!(p.to_string().parse::<Position>(), Ok(p));
    }

    /// Fuzz-lite: any text gives a value or a typed error, never a panic.
    #[test]
    fn parsers_never_panic(text in "\\PC{0,24}") {
        for e in [
            text.parse::<ModuleId>().err(),
            text.parse::<ParamAddr>().err(),
            text.parse::<ModSource>().err(),
            text.parse::<RouteAddr>().err(),
            text.parse::<Position>().err(),
        ].into_iter().flatten() {
            assert_typed(&e);
        }
    }

    /// Address-shaped junk.
    #[test]
    fn address_soup_never_panics(a in "[a-z_0-9]{0,6}", b in "[a-z_0-9\\[\\]]{0,8}", sep in prop::sample::select(vec![".", "->", "[", " -> ", ""])) {
        let text = format!("{a}{sep}{b}");
        for e in [
            text.parse::<ParamAddr>().err(),
            text.parse::<ModSource>().err(),
            text.parse::<RouteAddr>().err(),
            text.parse::<Position>().err(),
        ].into_iter().flatten() {
            assert_typed(&e);
        }
    }
}
