# content/presets

Sounds made in the plugin's editor, one patch document per file (`<name>.patch.json`). The first four are the owner's G1 sounds, "a bass, pad, pluck and lead you like" ([docs/p1b-owner.md](../../docs/p1b-owner.md), owner build 2). None are committed yet. The factory presets the owner picks from `content/presets-candidates/` go into `<type>/` folders (`cargo xtask preset-add --picks`).

`init.patch.json` is **Init** (lane AM, R1/S1): the patch a fresh plugin instance plays (`enth-plugin`'s `init` module embeds it) and the one `cargo xtask starter-project` loads. It is the starter patch, gain-staged: every P1 module, osc 1 into the filter and env 1 on the amp, the extras switched off, the master at −12 dB so a four-note chord peaks at or below −1 dBFS (a test in `crates/enth-plugin/src/init.rs` renders it). Change it like any preset, and run that test and `cargo xtask host-render --package enth-plugin` (its `chord` case is a fresh instance).

## How a sound becomes a file

1. Start from `cargo xtask starter-project --out ~/enth-g1/g1-sounds.rpp`: a REAPER project with four tracks, `bass`, `pad`, `pluck` and `lead`, each holding enth with Init (every P1 module, the extra ones switched off, as a fresh instance has it; the editor adds and removes modules from a card's "…" menu or the header's "+ Add"). Any project works: name each track after its sound.
2. Make the sound in the editor, then save the project (Ctrl+S). The plugin folds the host's parameter values into the patch when REAPER saves, so the project holds what you heard.
3. An agent runs, from the repo root:

   ```sh
   cargo xtask extract-presets ~/path/to/project.rpp            # --dry-run first to see the names
   ```

   Each enth instance (CLAP or VST3) becomes `content/presets/<name>.patch.json`, named after the FX's own name if you renamed it in REAPER, else the track: track `bass` gives `bass.patch.json` named "Bass". The file is the plugin's saved `patch` field with only its metadata changed: `meta.name` is that name, and a sound made from Init (the starter project's patch) loses Init's `init` tag and description and gains the type tag its name spells (`bass`, `pad`, `pluck`, `lead`, …), so the four G1 sounds sit beside the factory Init without a clash. A name another preset in the folder already has is refused; `--force` replaces a file that differs, `--name` names a single instance. A sound whose name is no type gets a note with the `preset-add` line that tags it.
4. The agent checks it (`cargo test -p enth-plugin --test presets`; `enth describe <file>` for a look at the sound) and commits it.

Do not edit or reformat the files by hand: the test checks that each one is exactly what the plugin saves (`Patch::to_json()`), so the plugin loads it and saves it back unchanged. To change a preset, open its project, edit the sound in the editor, save, and extract it again with `--force`.
