//! What an undo step did, in the editor's words, for the header's Undo and Redo tooltips ("Undo Filter 1 › Cutoff").

use enth_patch::history::Entry;
use enth_patch::{ModSource, ModuleId, Op, ParamAddr, Patch, Target};

use super::cards::view::source_name;
use crate::theme::glyph;

/// A step's name from its first op, against `patch` (the patch as it is now; titles come from module labels).
pub fn entry(patch: &Patch, entry: &Entry) -> String {
    entry
        .forward
        .first()
        .map_or_else(String::new, |op| describe(patch, op))
}

fn module(patch: &Patch, id: &ModuleId) -> String {
    source_name(patch, &ModSource::Module(id.clone()))
}

fn param(patch: &Patch, addr: &ParamAddr) -> String {
    let owner = match addr.target() {
        Target::Module(id) => module(patch, id),
        Target::Voice => "Voice".to_owned(),
        Target::Master => "Master".to_owned(),
    };
    let name = patch.spec(addr).map_or(addr.param(), |s| s.name);
    format!("{owner} {} {name}", glyph::PATH)
}

fn describe(patch: &Patch, op: &Op) -> String {
    match op {
        Op::SetParam { addr, .. } => param(patch, addr),
        Op::SetMacro { index, .. } | Op::SetMacroName { index, .. } => patch.macros[index.get()]
            .name
            .clone()
            .unwrap_or_else(|| format!("Macro {}", index.one_based())),
        Op::SetEnabled { id, enabled } => {
            let state = if *enabled { "on" } else { "off" };
            format!("{} {state}", module(patch, id))
        }
        Op::SetDepth { route, .. } | Op::RemoveRoute { route } => format!(
            "{} {} {}",
            source_name(patch, &route.src),
            glyph::SEND,
            param(patch, &route.dst)
        ),
        Op::InsertRoute { route, .. } => format!(
            "{} {} {}",
            source_name(patch, &route.src),
            glyph::SEND,
            param(patch, &route.dst)
        ),
        Op::SetSlot { slot, addr } => match addr {
            Some(addr) => format!(
                "automation slot {} {} {}",
                slot.one_based(),
                glyph::SEND,
                param(patch, addr)
            ),
            None => format!("automation slot {}", slot.one_based()),
        },
        Op::AddModule { module: m, .. } => format!("add {}", m.id.as_str()),
        Op::RemoveModule { id } => format!("remove {}", id.as_str()),
        Op::Move { id, .. } | Op::SetLabel { id, .. } | Op::SetAmpEnv { id } => module(patch, id),
        other => other.name().to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use enth_patch::history::History;

    #[test]
    fn steps_read_in_the_editors_words() {
        let mut patch = Patch::init();
        let mut history = History::default();
        let level: ParamAddr = "osc1.level".parse().unwrap();
        history
            .edit(
                &mut patch,
                vec![Op::SetParam {
                    addr: level,
                    value: 0.0,
                }],
                None,
            )
            .unwrap();
        let step = history.undo_stack().last().unwrap();
        assert_eq!(entry(&patch, step), "Osc 1 › Level");
        let off = Op::SetEnabled {
            id: ModuleId::new("osc1").unwrap(),
            enabled: false,
        };
        assert_eq!(describe(&patch, &off), "Osc 1 off");
    }
}
