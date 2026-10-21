//! Shared test support: the fixture patches and instance states, and small constructors.
#![allow(dead_code, reason = "each test binary uses a different part")]

use enth_patch::model::{ModuleInstance, Route, Section};
use enth_patch::{
    MacroIndex, ModSource, ModuleId, ModuleKind, Op, ParamAddr, Patch, PatchMeta, Position,
    SlotIndex, apply_all,
};
use enth_session::{InstanceState, MonitorState, UiState};

pub fn addr(text: &str) -> ParamAddr {
    text.parse().unwrap()
}

pub fn id(text: &str) -> ModuleId {
    ModuleId::new(text).unwrap()
}

pub fn slot(one_based: usize) -> SlotIndex {
    SlotIndex::from_one_based(one_based).unwrap()
}

pub fn macro_index(one_based: usize) -> MacroIndex {
    MacroIndex::from_one_based(one_based).unwrap()
}

/// The init patch as a load returns it: the file layer stamps an empty `engine` with its version on save.
pub fn init_patch() -> Patch {
    let mut p = Patch::init();
    enth_patch::file::ENGINE_VERSION.clone_into(&mut p.engine);
    p
}

/// The init patch plus a filter (`filter1`, labelled "Tone"), a per-voice LFO routed to its cutoff, non-default
/// values, three slots (first, second and last), a named macro and metadata: everything the saved state carries.
pub fn custom_patch() -> Patch {
    let mut p = init_patch();
    let ops = vec![
        Op::AddModule {
            at: Position {
                section: Section::VoiceFx,
                index: 0,
            },
            module: ModuleInstance::new(id("filter1"), ModuleKind::Svf),
        },
        Op::AddModule {
            at: Position {
                section: Section::Modulators,
                index: 1,
            },
            module: ModuleInstance::new(id("lfo1"), ModuleKind::Lfo),
        },
        Op::SetParam {
            addr: addr("filter1.cutoff"),
            value: 1234.5,
        },
        Op::SetParam {
            addr: addr("filter1.resonance"),
            value: 33.0,
        },
        Op::SetParam {
            addr: addr("voice.glide"),
            value: 120.0,
        },
        Op::InsertRoute {
            index: 0,
            route: Route {
                src: ModSource::Module(id("lfo1")),
                dst: addr("filter1.cutoff"),
                depth: 25.0,
            },
        },
        Op::SetSlot {
            slot: slot(1),
            addr: Some(addr("filter1.cutoff")),
        },
        Op::SetSlot {
            slot: slot(2),
            addr: Some(addr("master.level")),
        },
        Op::SetSlot {
            slot: slot(64),
            addr: Some(addr("voice.glide")),
        },
        Op::SetMacro {
            index: macro_index(1),
            value: 0.25,
        },
        Op::SetMacroName {
            index: macro_index(1),
            name: Some("Brightness".to_owned()),
        },
        Op::SetLabel {
            id: id("filter1"),
            label: Some("Tone".to_owned()),
        },
        Op::SetMeta {
            meta: PatchMeta {
                name: "S1s fixture".to_owned(),
                author: "enth".to_owned(),
                description: "Saved state v1 fixture: slots, macros, a label and a route"
                    .to_owned(),
                tags: vec!["fixture".to_owned()],
            },
        },
    ];
    apply_all(&mut p, &ops).unwrap();
    p
}

/// A non-default instance state.
pub fn custom_instance() -> InstanceState {
    InstanceState {
        monitor: MonitorState { fx_bypass: true },
        ui: UiState {
            layout: "lanes".to_owned(),
            scale_percent: 150,
            advanced: true,
        },
        game: None,
        preset: None,
        learn: None,
    }
}

/// The fixture cases of the saved state v1: (name, patch, instance state).
pub fn v1_cases() -> Vec<(&'static str, Patch, InstanceState)> {
    vec![
        ("default", init_patch(), InstanceState::default()),
        ("custom", custom_patch(), custom_instance()),
    ]
}
