//! The 72 host parameters (8 macros, 64 automation slots) with their permanent string ids, and the two persistent
//! fields (lane SK). The ids are permanent (docs/naming.md): hosts key saved projects and automation by them, and
//! nice-plug hashes them to the 31-bit ids hosts store (`macro_1` is 825615486, `slot_1` 1248030256, `slot_64`
//! 34232479). `#[nested(array)]` appends `_1`, `_2`, … to `macro` and `slot`, in the order of
//! [`enth_session::bridge::HOST_PARAM_IDS`] (a test pins both).
//!
//! Every parameter is a linear 0–1 `FloatParam` with no smoothing: the value is normalised, plain values live only
//! in the patch and the engine, and the engine smooths. Macros take CLAP polyphonic modulation (id = macro index);
//! slots do not. Names are "Macro N" and "Slot N": stock nice-plug fixes names at construction (lane FK's fork adds
//! renaming, lane PL wires it). Defaults: a macro 0, a slot 0.5 ([`enth_session::bridge::SLOT_DEFAULT_NORM`]).

use std::sync::Arc;

use enth_patch::model::{MACROS, SLOTS};
use enth_session::bridge::{MACRO_DEFAULT_NORM, SLOT_DEFAULT_NORM};
use enth_session::{HostParam, HostValues};
use nice_plug::prelude::*;

use crate::Shared;
use crate::state::{InstanceField, PatchField};

/// One macro knob, `macro_N`.
#[derive(Params)]
pub struct MacroParam {
    #[id = "macro"]
    pub value: FloatParam,
}

/// One automation slot, `slot_N`.
#[derive(Params)]
pub struct SlotParam {
    #[id = "slot"]
    pub value: FloatParam,
}

/// The host's view of the plugin: 72 parameters in host order (`macro_1..8`, then `slot_1..64`), and the saved
/// state's two fields, `enth` and `patch` (nice-plug sets them in that order, after the parameters).
#[derive(Params)]
pub struct EnthParams {
    #[nested(array)]
    pub macros: [MacroParam; MACROS],
    #[nested(array)]
    pub slots: [SlotParam; SLOTS],
    /// The patch, exactly `Patch::to_json()` (docs/session.md, "The saved state, version 1").
    #[persist = "patch"]
    pub patch: PatchField,
    /// The per-instance state outside the patch, `enth.plugin-state/1`.
    #[persist = "enth"]
    pub instance: InstanceField,
}

fn unit_range() -> FloatRange {
    FloatRange::Linear { min: 0.0, max: 1.0 }
}

impl EnthParams {
    /// The parameters, with persistent fields bound to `shared`'s session.
    pub fn new(shared: &Arc<Shared>) -> Self {
        let mut n = 0;
        let macros = [(); MACROS].map(|()| {
            n += 1;
            MacroParam {
                value: FloatParam::new(format!("Macro {n}"), MACRO_DEFAULT_NORM, unit_range())
                    .with_poly_modulation_id(n - 1),
            }
        });
        let mut n = 0;
        let slots = [(); SLOTS].map(|()| {
            n += 1;
            SlotParam {
                value: FloatParam::new(format!("Slot {n}"), SLOT_DEFAULT_NORM, unit_range()),
            }
        });
        EnthParams {
            macros,
            slots,
            patch: PatchField::new(Arc::clone(shared)),
            instance: InstanceField::new(Arc::clone(shared)),
        }
    }

    /// The nice-plug parameter behind a host parameter (the editor's `ParamSetter` needs it).
    pub fn param(&self, p: HostParam) -> &FloatParam {
        match p {
            HostParam::Macro(i) => &self.macros[i.get()].value,
            HostParam::Slot(s) => &self.slots[s.get()].value,
        }
    }

    /// The 72 unmodulated normalised values: what the session folds (main and GUI threads).
    pub fn host_values(&self) -> HostValues {
        let mut v = HostValues::default();
        for (out, m) in v.macros.iter_mut().zip(&self.macros) {
            *out = m.value.unmodulated_normalized_value();
        }
        for (out, s) in v.slots.iter_mut().zip(&self.slots) {
            *out = s.value.unmodulated_normalized_value();
        }
        v
    }

    /// The 72 modulated normalised values into `out`: what `process()` sends the engine, so the engine follows CLAP
    /// mono modulation while the patch keeps the unmodulated value. Atomic loads only (audio thread).
    pub fn read_modulated(&self, out: &mut HostValues) {
        for (o, m) in out.macros.iter_mut().zip(&self.macros) {
            *o = m.value.modulated_normalized_value();
        }
        for (o, s) in out.slots.iter_mut().zip(&self.slots) {
            *o = s.value.modulated_normalized_value();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use enth_session::bridge::HOST_PARAM_IDS;

    fn params() -> EnthParams {
        let plugin = crate::Enth::default();
        EnthParams::new(plugin.shared())
    }

    #[test]
    fn the_host_sees_72_params_with_the_permanent_ids_and_names() {
        let params = params();
        let map = params.param_map();
        assert_eq!(map.len(), 72);
        for ((id, ptr, group), expected) in map.iter().zip(HOST_PARAM_IDS) {
            assert_eq!(id, expected);
            assert_eq!(group, "", "{id} must not be grouped");
            // SAFETY: the pointer points into `params`, which outlives this loop.
            let name = unsafe { ptr.name().to_owned() };
            let (kind, n) = id.split_once('_').unwrap();
            let kind = if kind == "macro" { "Macro" } else { "Slot" };
            assert_eq!(name, format!("{kind} {n}"));
            assert_eq!(HostParam::from_id(id).map(HostParam::id), Some(expected));
        }
    }

    #[test]
    fn the_hashed_ids_hosts_store_are_pinned() {
        // docs/naming.md; REAPER's idents are `<index>:<hash>`. nice-plug 0.4.2's `wrapper::util::hash_param_id`
        // (private): a Rabin fingerprint with the top bit cleared (VST3 reserves it for the host).
        fn hash_param_id(id: &str) -> u32 {
            let hash = id
                .bytes()
                .fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(u32::from(b)));
            hash & !(1 << 31)
        }
        assert_eq!(hash_param_id("macro_1"), 825_615_486);
        assert_eq!(hash_param_id("slot_1"), 1_248_030_256);
        assert_eq!(hash_param_id("slot_64"), 34_232_479);
    }

    #[test]
    fn only_macros_take_polyphonic_modulation_and_defaults_match_the_bridge() {
        let params = params();
        for (n, m) in params.macros.iter().enumerate() {
            assert_eq!(m.value.poly_modulation_id(), Some(n as u32));
            assert_eq!(
                m.value.default_normalized_value().to_bits(),
                MACRO_DEFAULT_NORM.to_bits()
            );
        }
        for s in &params.slots {
            assert_eq!(s.value.poly_modulation_id(), None);
            assert_eq!(
                s.value.default_normalized_value().to_bits(),
                SLOT_DEFAULT_NORM.to_bits()
            );
        }
        assert_eq!(params.host_values(), HostValues::default());
        let mut modulated = HostValues {
            macros: [9.0; MACROS],
            slots: [9.0; SLOTS],
        };
        params.read_modulated(&mut modulated);
        assert_eq!(modulated, HostValues::default());
    }

    #[test]
    fn param_lookup_follows_the_dense_order() {
        let params = params();
        for p in HostParam::all() {
            let ptr = params.param(p).as_ptr();
            let (id, _, _) = params
                .param_map()
                .into_iter()
                .find(|(_, q, _)| *q == ptr)
                .unwrap();
            assert_eq!(id, p.id());
        }
    }

    #[test]
    fn the_saved_state_has_the_two_permanent_fields() {
        let params = params();
        let fields = params.serialize_fields();
        assert_eq!(
            fields.keys().map(String::as_str).collect::<Vec<_>>(),
            ["enth", "patch"]
        );
    }
}
