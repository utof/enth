# content/presets

Sounds made in the plugin's editor, one patch document per file (`<name>.patch.json`). The first four are the owner's G1 sounds, "a bass, pad, pluck and lead you like" ([docs/p1b-owner.md](../../docs/p1b-owner.md), owner build 2). None are committed yet.

## How a sound becomes a file

1. In REAPER, put enth on a track and name the track after the sound (`bass`, `pad`, `pluck`, `lead`). Several tracks in one project are fine.
2. Make the sound in the editor, then save the project (Ctrl+S). The plugin folds the host's parameter values into the patch when REAPER saves, so the project holds what you heard.
3. An agent runs, from the repo root:

   ```sh
   cargo xtask extract-presets ~/path/to/project.rpp            # --dry-run first to see the names
   ```

   Each enth instance (CLAP or VST3) becomes `content/presets/<name>.patch.json`, named after the FX's own name if you renamed it in REAPER, else the track. The file is the plugin's saved `patch` field, byte for byte; `--force` replaces a file that differs, `--name` names a single instance.
4. The agent checks it (`cargo test -p enth-plugin --test presets`; `enth describe <file>` for a look at the sound) and commits it.

Do not edit or reformat the files by hand: the test checks that each one is exactly what the plugin saves (`Patch::to_json()`), so the plugin loads it and saves it back unchanged. To change a preset, open its project, edit the sound in the editor, save, and extract it again with `--force`.
