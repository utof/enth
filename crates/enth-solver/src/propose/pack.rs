//! **Owner: lane ONX** (freeze point F3s). Pre-declared by lane S0s (R3); empty until ONX writes it.
//!
//! The model file's reader (docs/r3.md, "The model file"): `ModelPack::parse(&[u8])` reads the versioned header
//! (format version, model id, template id and version, the ordered parameter addresses and their normalisation, the
//! feature config, the engine version) and checks the sha256 (`sha2` is declared in this crate's manifest). A newer
//! format is refused like `code_too_new`; a header that does not match this build's catalogue and template means
//! "Create from sound" runs R3a and says so. Fixture tests pin the layout.
