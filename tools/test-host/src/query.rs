//! What a plugin says about itself: descriptor, parameters, ports, extensions. Feeds `info` and the
//! name-to-id lookups the other commands use.

use clack_extensions::audio_ports::{AudioPortInfoBuffer, PluginAudioPorts};
use clack_extensions::gui::PluginGui;
use clack_extensions::note_ports::{NotePortInfoBuffer, PluginNotePorts};
use clack_extensions::params::{ParamInfoBuffer, PluginParams};
use clack_extensions::state::PluginState;
use clack_host::prelude::*;
use serde_json::{Value, json};

use crate::host::Host;

#[derive(Clone, Debug)]
pub struct ParamDesc {
    pub index: u32,
    pub id: u32,
    pub name: String,
    pub module: String,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    pub flags: Vec<&'static str>,
    pub value: Option<f64>,
}

impl ParamDesc {
    pub fn to_json(&self) -> Value {
        json!({
            "index": self.index,
            "id": self.id,
            "name": self.name,
            "module": self.module,
            "range": { "min": self.min, "max": self.max, "default": self.default },
            "value": self.value,
            "flags": self.flags,
        })
    }
}

#[derive(Clone, Debug)]
pub struct AudioPortDesc {
    pub id: u32,
    pub name: String,
    pub channels: u32,
    pub main: bool,
    pub port_type: Option<String>,
}

impl AudioPortDesc {
    fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "name": self.name,
            "channels": self.channels,
            "main": self.main,
            "type": self.port_type,
        })
    }
}

fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn cstr(s: Option<&std::ffi::CStr>) -> Option<String> {
    s.map(|s| s.to_string_lossy().into_owned())
}

pub fn params(instance: &mut PluginInstance<Host>) -> Vec<ParamDesc> {
    let handle = instance.plugin_handle();
    let Some(ext) = handle.get_extension::<PluginParams>() else {
        return Vec::new();
    };
    let mut buffer = ParamInfoBuffer::new();
    let mut out = Vec::new();
    for index in 0..ext.count(&handle) {
        let Some(info) = ext.get_info(&handle, index, &mut buffer) else {
            continue;
        };
        out.push(ParamDesc {
            index,
            id: info.id.get(),
            name: lossy(info.name),
            module: lossy(info.module),
            min: info.min_value,
            max: info.max_value,
            default: info.default_value,
            flags: info.flags.iter_names().map(|(n, _)| n).collect(),
            value: ext.get_value(&handle, info.id),
        });
    }
    out
}

/// Finds a parameter by exact name or by numeric id.
pub fn resolve_param(params: &[ParamDesc], key: &str) -> Option<u32> {
    params
        .iter()
        .find(|p| p.name == key)
        .map(|p| p.id)
        .or_else(|| {
            let id = key.parse::<u32>().ok()?;
            params.iter().find(|p| p.id == id).map(|p| p.id)
        })
}

/// `(inputs, outputs)`.
pub fn audio_ports(
    instance: &mut PluginInstance<Host>,
) -> (Vec<AudioPortDesc>, Vec<AudioPortDesc>) {
    let handle = instance.plugin_handle();
    let Some(ext) = handle.get_extension::<PluginAudioPorts>() else {
        return (Vec::new(), Vec::new());
    };
    let mut buffer = AudioPortInfoBuffer::new();
    let mut side = |is_input: bool| {
        (0..ext.count(&handle, is_input))
            .filter_map(|i| {
                let info = ext.get(&handle, i, is_input, &mut buffer)?;
                Some(AudioPortDesc {
                    id: info.id.get(),
                    name: lossy(info.name),
                    channels: info.channel_count,
                    main: info.flags.iter_names().any(|(n, _)| n == "IS_MAIN"),
                    port_type: info.port_type.map(|t| t.0.to_string_lossy().into_owned()),
                })
            })
            .collect::<Vec<_>>()
    };
    let inputs = side(true);
    let outputs = side(false);
    (inputs, outputs)
}

fn note_ports(instance: &mut PluginInstance<Host>) -> (Vec<Value>, Vec<Value>) {
    let handle = instance.plugin_handle();
    let Some(ext) = handle.get_extension::<PluginNotePorts>() else {
        return (Vec::new(), Vec::new());
    };
    let mut buffer = NotePortInfoBuffer::new();
    let mut side = |is_input: bool| {
        (0..ext.count(&handle, is_input))
            .filter_map(|i| {
                let info = ext.get(&handle, i, is_input, &mut buffer)?;
                Some(json!({
                    "id": info.id.get(),
                    "name": lossy(info.name),
                    "supported_dialects": info.supported_dialects.iter_names().map(|(n, _)| n).collect::<Vec<_>>(),
                    "preferred_dialect": info.preferred_dialect.map(|d| format!("{d:?}")),
                }))
            })
            .collect::<Vec<_>>()
    };
    let inputs = side(true);
    let outputs = side(false);
    (inputs, outputs)
}

fn extensions(instance: &mut PluginInstance<Host>) -> Value {
    let handle = instance.plugin_handle();
    json!({
        "params": handle.get_extension::<PluginParams>().is_some(),
        "audio-ports": handle.get_extension::<PluginAudioPorts>().is_some(),
        "note-ports": handle.get_extension::<PluginNotePorts>().is_some(),
        "state": handle.get_extension::<PluginState>().is_some(),
        "gui": handle.get_extension::<PluginGui>().is_some(),
    })
}

/// The full `info` report for one plugin of `entry`.
pub fn plugin_report(entry: &PluginEntry, id: &std::ffi::CStr) -> Result<Value, String> {
    let factory = entry
        .get_plugin_factory()
        .ok_or("the library has no CLAP plugin factory")?;
    let descriptor = factory
        .plugin_descriptors()
        .find(|d| d.id() == Some(id))
        .ok_or("plugin vanished from the factory")?;
    let features: Vec<String> = descriptor
        .features()
        .map(|f| f.to_string_lossy().into_owned())
        .collect();
    let descriptor_json = json!({
        "id": cstr(descriptor.id()),
        "name": cstr(descriptor.name()),
        "vendor": cstr(descriptor.vendor()),
        "version": cstr(descriptor.version()),
        "description": cstr(descriptor.description()),
        "url": cstr(descriptor.url()),
        "manual_url": cstr(descriptor.manual_url()),
        "support_url": cstr(descriptor.support_url()),
        "features": features,
    });

    let mut instance = crate::host::instantiate(entry, id)?;
    let params = params(&mut instance);
    let (audio_in, audio_out) = audio_ports(&mut instance);
    let (note_in, note_out) = note_ports(&mut instance);
    let ext = extensions(&mut instance);
    Ok(json!({
        "descriptor": descriptor_json,
        "params": params.iter().map(ParamDesc::to_json).collect::<Vec<_>>(),
        "audio_ports": {
            "inputs": audio_in.iter().map(AudioPortDesc::to_json).collect::<Vec<_>>(),
            "outputs": audio_out.iter().map(AudioPortDesc::to_json).collect::<Vec<_>>(),
        },
        "note_ports": { "inputs": note_in, "outputs": note_out },
        "extensions": ext,
    }))
}
