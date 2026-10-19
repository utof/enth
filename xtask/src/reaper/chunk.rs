//! The plugin state as REAPER hands it out and takes it: base64 around nice-plug's JSON state, in a layout that
//! depends on the format. The scenarios load a patch with [`write_state_chunk`] and read their dumps with [`Saved`].

use std::path::Path;

use super::Format;

pub fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for (i, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            if i <= chunk.len() {
                out.push(char::from(TABLE[((n >> shift) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

pub fn base64_decode(text: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let (mut bits, mut count) = (0u32, 0);
    for byte in text.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => continue,
        };
        bits = (bits << 6) | u32::from(value);
        count += 6;
        if count >= 8 {
            count -= 8;
            out.push((bits >> count) as u8);
            bits &= (1 << count) - 1;
        }
    }
    out
}

/// Finds nice-plug's JSON state (`{"version":…`) inside the host's chunk, which wraps it in a
/// format-specific header, and parses the first JSON value there.
pub fn nice_plug_state(chunk: &[u8]) -> Result<serde_json::Value, String> {
    let start = chunk
        .windows(10)
        .position(|w| w == br#"{"version""#)
        .ok_or("no nice-plug state JSON in the chunk")?;
    serde_json::Deserializer::from_slice(&chunk[start..])
        .into_iter::<serde_json::Value>()
        .next()
        .ok_or("empty state")?
        .map_err(|e| format!("bad state JSON: {e}"))
}

/// Writes `patch` as a nice-plug state chunk that REAPER sets through its `clap_chunk` / `vst_chunk` named config
/// parameter (`common.lua`'s `inject_state`), base64, and returns the file's path. The layouts are the ones REAPER
/// 7.82 saves for nice-plug 0.4.2 (measured): CLAP is the plugin's own stream, a `u64` length then the state JSON;
/// VST3 is a `u32` length, a `u32` 1, the JSON and 8 zero bytes. The state holds no parameters, only the `patch`
/// field (the parameters keep their values; the patch wins).
pub fn write_state_chunk(
    out: &Path,
    name: &str,
    format: Format,
    patch: &str,
) -> Result<String, String> {
    let state =
        serde_json::json!({ "version": "0.0.0", "params": {}, "fields": { "patch": patch } });
    let json = serde_json::to_vec(&state).map_err(|e| format!("state chunk: {e}"))?;
    let mut bytes = Vec::with_capacity(json.len() + 16);
    match format {
        Format::Clap => bytes.extend_from_slice(&(json.len() as u64).to_le_bytes()),
        Format::Vst3 => {
            let len = u32::try_from(json.len()).map_err(|_| "state chunk too large")?;
            bytes.extend_from_slice(&len.to_le_bytes());
            bytes.extend_from_slice(&1u32.to_le_bytes());
        }
    }
    bytes.extend_from_slice(&json);
    if format == Format::Vst3 {
        bytes.extend_from_slice(&[0; 8]);
    }
    let path = out.join(format!("chunk-{}-{name}-in.b64", format.tag()));
    std::fs::write(&path, base64_encode(&bytes)).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path.display().to_string())
}

/// A state a script dumped with `dump_chunk(label, …)`: the plugin's saved state at that moment, decoded.
pub struct Saved(pub serde_json::Value);

impl Saved {
    pub fn read(out: &Path, format: Format, label: &str) -> Result<Saved, String> {
        let path = out.join(format!("chunk-{}-{label}.b64", format.tag()));
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        nice_plug_state(&base64_decode(&text)).map(Saved)
    }

    /// The host parameter's saved value (nice-plug stores `{"f32": v}`).
    pub fn param(&self, id: &str) -> Option<f64> {
        self.0["params"][id]["f32"].as_f64()
    }

    /// The `patch` field's text: exactly `Patch::to_json()`.
    pub fn patch_text(&self) -> Option<&str> {
        self.0["fields"]["patch"].as_str()
    }

    pub fn patch(&self) -> Option<serde_json::Value> {
        serde_json::from_str(self.patch_text()?).ok()
    }

    /// The `enth` field's text (monitor bypass, editor preferences).
    pub fn instance_text(&self) -> &str {
        self.0["fields"]["enth"].as_str().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trips_every_length() {
        for len in 0..40usize {
            let bytes: Vec<u8> = (0..len).map(|i| (i * 37 + 11) as u8).collect();
            assert_eq!(base64_decode(&base64_encode(&bytes)), bytes, "length {len}");
        }
    }

    #[test]
    fn state_json_is_found_after_either_header() {
        let patch = r#"{"schema":1}"#;
        for format in [Format::Clap, Format::Vst3] {
            let dir = std::env::temp_dir().join(format!(
                "enth-reaper-chunk-test-{}-{}",
                std::process::id(),
                format.tag()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            let path = write_state_chunk(&dir, "t", format, patch).unwrap();
            let text = std::fs::read_to_string(path).unwrap();
            let state = nice_plug_state(&base64_decode(&text)).unwrap();
            assert_eq!(state["fields"]["patch"], patch);
            std::fs::remove_dir_all(&dir).unwrap();
        }
    }
}
