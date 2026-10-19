//! `PatchFile` → `Patch` for golden cases, through enth-patch's file layer (lane S1b), which also checks every value
//! against its `ParamSpec`. A case embeds its patch as a `[source.patch]` table with the shape of a `*.patch.json`
//! file ([`enth_patch::file::PatchFile`], written in TOML).

use enth_patch::Patch;
use enth_patch::file::PatchFile;

/// Convert a patch file (already parsed) into a `Patch`.
pub fn patch_from_file(file: &PatchFile) -> Result<Patch, String> {
    file.to_patch().map_err(|e| e.to_string())
}
