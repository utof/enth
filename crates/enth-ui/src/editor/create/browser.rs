//! **Owner: lane PUI** (the Create view, feature `create`). Pre-declared by lane S0s (R3); empty until PUI writes it.
//!
//! The in-editor folder browser, the fallback where a file drop does not reach the editor (docs/r3.md, "File input"):
//! no `rfd`, which would need a portal or a GTK backend on Linux. Whether REAPER drags media items into a plugin
//! window as files is an open check (SK3 or FK3), and this browser is why that check is not blocking. Directory
//! listing is the shell's I/O (`enth-plugin`'s `create.rs`), handed over as data. Sizes go in `theme/sizes/create.rs`.
