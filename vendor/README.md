# vendor/

Privately vendored copies of nice-plug and baseview, behind `[patch.crates-io]` in the root `Cargo.toml`. Lane FK of P1b owns this directory; no other lane edits it (a lane that needs a change asks for it).

Why vendored and not a git fork: a public fork would expose the project's name, CI would depend on Codeberg being up, and `deny.toml` would need `allow-git`. Decision and measurements: `docs/adr/0001-plugin-shell.md` and `docs/adr/0002-ui-toolkit.md` (notes of 2026-10-19), research session `_local/sessions/2026-10-19-p1b-plugin-editor/`.

The directory is outside the cargo workspace (`exclude = ["vendor"]` in the root `Cargo.toml`), so `cargo fmt`, `cargo clippy` and `--workspace` never touch it.

## Crates

Each package is the unchanged content of its crates.io `.crate` file at the version `Cargo.lock` pins. The bytes were taken from cargo's registry cache; each file's sha256 is the checksum `Cargo.lock` records (and the crates.io index serves), so they are the files crates.io hands out. Every entry keeps its crates.io version, so the exact `=` pins in the root `Cargo.toml` still match.

| Directory | Crate | Version | Licence | Source | sha256 of the `.crate` | Upstream commit (`.cargo_vcs_info.json`) |
| --- | --- | --- | --- | --- | --- | --- |
| `nice-plug/crates/nice-plug` | nice-plug | 0.4.2 | ISC | <https://static.crates.io/crates/nice-plug/nice-plug-0.4.2.crate> | `3f47be74bed3e8aa17bb055d4ff89af1fb016aa0d2756cd9b6c029a3743676a5` | `91ba75d20c43ca0cd21afa802d0ae8c0511f48ed` (Codeberg RustAudio/nice-plug, `crates/nice-plug`) |
| `nice-plug/crates/nice-plug-core` | nice-plug-core | 0.4.2 | ISC | <https://static.crates.io/crates/nice-plug-core/nice-plug-core-0.4.2.crate> | `ab8fe3f3e395c1140d7519a78069b59b2e3bf6da2b904d0c54ad7ab49a60ba09` | `91ba75d20c43ca0cd21afa802d0ae8c0511f48ed` (`crates/nice-plug-core`) |
| `nice-plug/crates/nice-plug-egui` | nice-plug-egui | 0.6.0 | ISC | <https://static.crates.io/crates/nice-plug-egui/nice-plug-egui-0.6.0.crate> | `54dd608bf12c94c7bdab22cd41122cd0e910454a2e6e59bd6a46ca0fb000c90e` | `faa0cd61d249bf7c5103f3072be0c65cf72e9f4e` (`crates/nice-plug-egui`) |
| `baseview` | baseview | 0.4.1 | MIT OR Apache-2.0 | <https://static.crates.io/crates/baseview/baseview-0.4.1.crate> | `4179c1b5e41cec78a6b000eaa8317ddbc2556124dabc7344ff1df89d566cc517` | `8c6e6bab4b3c10f34b23dc12e878814349578777` (GitHub RustAudio/baseview) |
| `egui-baseview` | egui-baseview | 0.9.0 | MIT OR Apache-2.0 | <https://static.crates.io/crates/egui-baseview/egui-baseview-0.9.0.crate> | `de6df8bd2f1c6b481ab6c029f741980d1a7b409053178fb9459053ae457828fe` | `1f645a35d25fd63784dcf2f91c6d72d467c6036b` (Codeberg RustAudio/egui-baseview) |

`nice-plug-derive` 0.1.2 and every other dependency stay on crates.io, unpatched.

Why egui-baseview is here: nice-plug-egui depends on it, and the keyboard-focus patch edits it. A `[patch.crates-io]` entry replaces a crate everywhere in the graph, so baseview (under egui-baseview) is the vendored one too.

## Licences

- nice-plug, nice-plug-core, nice-plug-egui: ISC, Copyright (c) 2022-2024 Robbert van der Helm. The crates.io packages carry no licence file; `LICENSE` in each of the three directories (added in the commit after the pristine import, the only file besides this README and `check-pristine.sh` that is not in a package) is the text of the repository's `LICENSE` at the Codeberg tag `v0.4.2` (fetched 2026-10-19).
- baseview, egui-baseview: `LICENSE-MIT` and `LICENSE-APACHE`, unchanged from the packages.
- Patch lines carry the licence of the crate they edit, so a human could still send them upstream.
- Entries in `THIRD_PARTY_NOTICES.md`: the project distributes these crates (compiled in) with local changes, so the file records them with the changes.

## Patches

Everything here is one git commit per change, after a single pristine-import commit. `git log --oneline -- vendor` is the list, and `vendor/check-pristine.sh` (`--full` for the whole diff) compares the tree with the `.crate` files in cargo's registry cache and checks their sha256 against the table above.

| # | Crate | Change | Size | Upstream status (2026-10-19) |
| --- | --- | --- | --- | --- |
| | | None yet | | |
