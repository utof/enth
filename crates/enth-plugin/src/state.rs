//! The persistent fields that bridge nice-plug's saved state and the session blob, `enth.plugin-state/1`
//! (docs/session.md, "The saved state, version 1"; lane SK).
//!
//! nice-plug saves `{"version", "params": {id: {"f32": v}}, "fields": {name: "<text>"}}`. The two fields are
//! permanent names (docs/naming.md):
//!
//! - `patch`: exactly `Patch::to_json()`, carried as a [`RawValue`] so it is neither escaped twice nor re-parsed
//!   through `serde_json::Value` (which would sort keys and widen floats). A `.patch.json` file and this field are
//!   interchangeable: the test host's `--state some.patch.json` puts the file's text here.
//! - `enth`: the per-instance state outside the patch (monitor bypass, editor preferences).
//!
//! Both run on the **host main thread** (nice-plug's state save and load) and never on the audio thread: the editor
//! never calls `GuiContext::set_state`. A load sets the parameters first, then `enth`, then `patch`; the patch wins
//! (the session neither folds the restored host values during the load nor lets the engine keep remembered host
//! slot values), and [`crate::Shared::rebaseline`] makes `process()` adopt the restored host values without sending
//! them. A save folds the host's values into the patch first (only values that changed since the session last saw
//! them, and only on a true difference), so the saved parameters and the saved patch agree.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use nice_plug::params::persist::PersistentField;
use serde_json::value::RawValue;

use crate::Shared;

/// The `patch` field.
pub struct PatchField {
    shared: Arc<Shared>,
}

impl PatchField {
    pub(crate) fn new(shared: Arc<Shared>) -> Self {
        PatchField { shared }
    }
}

/// The `enth` field.
pub struct InstanceField {
    shared: Arc<Shared>,
}

impl InstanceField {
    pub(crate) fn new(shared: Arc<Shared>) -> Self {
        InstanceField { shared }
    }
}

/// Wrap field text as raw JSON. The session's texts are JSON by construction (`Patch::to_json`, the instance
/// state, or a kept field that came in as JSON); anything else falls back to `fallback`'s text.
fn raw(text: String, fallback: impl FnOnce() -> String) -> Box<RawValue> {
    RawValue::from_string(text).unwrap_or_else(|_| {
        RawValue::from_string(fallback()).unwrap_or_else(|_| {
            // `null` always parses.
            RawValue::from_string("null".to_owned()).unwrap_or_default()
        })
    })
}

impl PersistentField<'_, Box<RawValue>> for PatchField {
    /// A host state load of the `patch` field: the session loads it (I7; a field that does not load leaves the init
    /// patch playing and is kept as text) and sends its plan at once.
    fn set(&self, new_value: Box<RawValue>) {
        // Before the plan goes out: a `process()` that already saw the restored values sends them to the old plan,
        // whose remembered slot values the new plan forgets; any later one adopts them silently.
        self.shared.rebaseline.store(true, Ordering::Release);
        let (_report, host) = self.shared.with_session(|s| s.load_patch(new_value.get()));
        debug_assert_eq!(host.len(), 0, "a load makes no host writes");
    }

    /// A host state save: fold the host's values, then the patch text (or the kept text of a patch that did not
    /// load, unchanged).
    fn map<F, R>(&self, f: F) -> R
    where
        F: Fn(&Box<RawValue>) -> R,
    {
        let host = self.shared.host_values();
        let (text, _) = self.shared.with_session(|s| s.save_patch(host.as_ref()));
        let value = raw(text, || {
            let s = self.shared.session.lock();
            s.patch().to_json()
        });
        f(&value)
    }
}

impl PersistentField<'_, Box<RawValue>> for InstanceField {
    /// A host state load of the `enth` field (before `patch`): the defaults if it does not load.
    fn set(&self, new_value: Box<RawValue>) {
        let (_report, host) = self
            .shared
            .with_session(|s| s.load_instance(new_value.get()));
        debug_assert_eq!(host.len(), 0, "a load makes no host writes");
    }

    fn map<F, R>(&self, f: F) -> R
    where
        F: Fn(&Box<RawValue>) -> R,
    {
        let text = self.shared.session.lock().save_instance();
        let value = raw(text, || enth_session::InstanceState::default().to_json());
        f(&value)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use enth_patch::Patch;
    use nice_plug::prelude::Params;

    use crate::Enth;

    fn custom_patch_json() -> String {
        let mut p = Patch::init();
        let osc = p.generators[0].id.clone();
        enth_patch::apply(
            &mut p,
            enth_patch::Op::SetParam {
                addr: enth_patch::ParamAddr::module(osc.clone(), "level").unwrap(),
                value: 37.5,
            },
        )
        .unwrap();
        enth_patch::apply(
            &mut p,
            enth_patch::Op::SetSlot {
                slot: enth_patch::SlotIndex::from_one_based(1).unwrap(),
                addr: Some(enth_patch::ParamAddr::module(osc, "level").unwrap()),
            },
        )
        .unwrap();
        p.to_json()
    }

    #[test]
    fn the_patch_field_is_exactly_the_patch_json() {
        let plugin = Enth::default();
        let fields = plugin.host_params().serialize_fields();
        let session_text = plugin.shared().session.lock().patch().to_json();
        assert_eq!(fields["patch"], session_text);
        assert_eq!(
            fields["enth"],
            r#"{"schema":"enth.plugin-state/1","monitor":{"fx_bypass":false},"ui":{"layout":"lanes","scale_percent":100,"advanced":false}}"#
        );
    }

    #[test]
    fn the_state_round_trips_to_an_identical_patch_and_identical_bytes() {
        let text = custom_patch_json();
        let first = Enth::default();
        let mut fields = BTreeMap::new();
        fields.insert("patch".to_owned(), text.clone());
        fields.insert(
            "enth".to_owned(),
            r#"{"schema":"enth.plugin-state/1","monitor":{"fx_bypass":true}}"#.to_owned(),
        );
        first.host_params().deserialize_fields(&fields);
        let saved = first.host_params().serialize_fields();
        assert_eq!(saved["patch"], text, "the patch comes back byte for byte");
        assert!(
            first
                .shared()
                .monitor_bypass
                .load(std::sync::atomic::Ordering::Acquire)
        );
        assert!(
            first
                .shared()
                .rebaseline
                .load(std::sync::atomic::Ordering::Acquire)
        );

        let second = Enth::default();
        second.host_params().deserialize_fields(&saved);
        let again = second.host_params().serialize_fields();
        assert_eq!(again, saved, "a second save is byte-identical");
        let a = first.shared().session.lock().patch().clone();
        let b = second.shared().session.lock().patch().clone();
        assert!(enth_patch::canon::identical(&a, &b));
        assert_eq!(a.slots, b.slots);
    }

    #[test]
    fn a_patch_that_does_not_load_is_kept_and_saved_back() {
        let plugin = Enth::default();
        let newer = r#"{"schema":999,"future":true}"#;
        let mut fields = BTreeMap::new();
        fields.insert("patch".to_owned(), newer.to_owned());
        plugin.host_params().deserialize_fields(&fields);
        assert!(plugin.shared().session.lock().status().patch_unloaded);
        assert_eq!(plugin.host_params().serialize_fields()["patch"], newer);
    }

    #[test]
    fn a_damaged_field_never_panics() {
        let plugin = Enth::default();
        // Text that is not JSON never reaches `set` (nice-plug's deserialisation refuses it first, a debug assertion
        // in its derive under `cfg(test)`); JSON that is not a patch or an instance state does.
        let mut fields = BTreeMap::new();
        fields.insert("patch".to_owned(), "[1,2,3]".to_owned());
        fields.insert(
            "enth".to_owned(),
            "{\"schema\":\"enth.plugin-state/99\"}".to_owned(),
        );
        plugin.host_params().deserialize_fields(&fields);
        let saved = plugin.host_params().serialize_fields();
        assert_eq!(
            saved["patch"], "[1,2,3]",
            "kept as text until the first edit"
        );
        assert!(saved["enth"].starts_with(r#"{"schema":"enth.plugin-state/1""#));
    }
}
