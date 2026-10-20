//! **Owner: lane PBC** (the preset core). Pre-declared empty by lane S0r (R1/S1).
//!
//! The plugin's preset I/O (report 03 §2.3, summary "Presets"): the user folder (`Documents/enth/Presets`,
//! overridden by `ENTH_PRESETS_DIR`), scanning `.patch.json` files lazily when the browser opens, and writing a saved
//! preset as exactly `Patch::to_json()`. The format, index and search live in `enth_session::presets`; the session
//! does no file I/O. Report 03 picked the `dirs` crate for the folder, but it pulls an MPL-2.0 crate, so it waits
//! for the owner (root Cargo.toml).
