# Naming

**Working name: `enth`** (the repo folder). The product name is open. The brief: *sound + progress* — so good you push through being stuck, discover ideas, and go beyond.

## Shortlist

| Name | Language | Meaning and fit | Collision check (2026-10) |
| --- | --- | --- | --- |
| **Shuhari** (守破離) | Japanese | The three stages of mastery: *shu* follow the form, *ha* break it, *ri* go beyond it. Maps to Learn → Play/Recreate → Create | No audio product found |
| **Slancio** | Italian | *Con slancio*: a performance marking meaning "with impetus, élan" — musical *and* forward drive | No audio product found |
| **Yamabiko** (山彦) | Japanese | Mountain echo; the spirit that answers by imitating your voice — fits sound matching | No audio product found |
| **Transonic** | English | Pushing through the sound barrier | No audio plugin found |
| **Risset** | French (person) | Jean-Claude Risset: the endless-rising Shepard-Risset glissando, and "analysis by synthesis" — rebuilding real sounds to understand them | Oli Larkin's Endless Series has a plugin named Risset; a real person's name — check with the estate |
| Échappée | French | Escape tone in music theory; a breakaway in cycling | Not checked |
| Essor | French | Soaring, taking off | Not checked |
| Prokopē | Greek | Stoic "progress", cutting your way forward | Not checked; obscure |
| Headroom | English | Space to grow above the signal | Not checked |
| Mimesis | Greek | Imitation — the matching loop | No synth found; Make Noise's Mimeophon shares the root |
| Ferrite | English | Iron-based like Rust; ferrite cores in audio transformers | Not checked |

## Taken in audio — avoid

Koe (Koe Recast voice changer), Neiro (Neiro Audio plugins; Miselu Neiro synth), Lyrebird (u-he effect; Reaktor synth), Patina (two plugins), Tessera (Audio Damage), Riser (AIR The Riser), Loom (AIR Loom II), Primer (Syntorial's synth), Mynth (Fingerlab, close to Myna).

## If the name is Shuhari: mode names

| Stage | Mode | What you do |
| --- | --- | --- |
| Shu 守 | Learn | Follow lessons; rebuild hidden patches with few knobs |
| Ha 破 | Play / Recreate | Break the form: the full synth, the Recreate game, duels |
| Ri 離 | Create | Leave the form: your own sounds, sound → patch, agent-assisted design |

## Before committing

- [ ] crates.io: is `<name>` or `<name>-dsp` taken?
- [ ] GitHub / Codeberg org names
- [ ] Domain (`.audio`, `.dev`, `.app`)
- [ ] KVR product search and a general web search with "plugin" / "synth"
- [ ] Trademark search (EUIPO, USPTO) in class 9 (software)
- [ ] Say it out loud to a few producers

Renaming the code later: the crate prefix `enth-` and the binary `enth` are the only places the name appears in code.

## Plugin identifiers (permanent)

Hosts key saved projects, presets and automation by these ids, so **once a build that uses one has left the machine it never changes**, even if the product is renamed. They are opaque strings and bytes, not names: `dev.enth` is a namespace, not a claim on a domain, and the working name `enth` can change without touching any of them. Chosen 2026-10-19 in the shell spike (Task 3).

| Id | Value | Used by |
| --- | --- | --- |
| CLAP id, shell spike | `dev.enth.spike-shell` | `spikes/spike-shell`; throwaway, never ships |
| VST3 class id, shell spike | ASCII `EnthSpikeShell01` = `45 6E 74 68 53 70 69 6B 65 53 68 65 6C 6C 30 31` | `spikes/spike-shell`; never ships |
| CLAP id, UI spike | `dev.enth.spike-ui` | `spikes/spike-ui-plugin` (the reference panel); throwaway, never ships |
| VST3 class id, UI spike | ASCII `EnthSpikeUiPnl01` = `45 6E 74 68 53 70 69 6B 65 55 69 50 6E 6C 30 31` | `spikes/spike-ui-plugin`; never ships |
| **CLAP id, product (reserved)** | `dev.enth.synth` | the real synth, `crates/plugin`, from Phase 1; not used anywhere yet |
| **VST3 class id, product (reserved)** | `55555EAD-AA77-C9FA-F878-DDDDB0B9D9BD`, random, as nice-plug bytes `[0x55, 0x55, 0x5E, 0xAD, 0xAA, 0x77, 0xC9, 0xFA, 0xF8, 0x78, 0xDD, 0xDD, 0xB0, 0xB9, 0xD9, 0xBD]` | the real synth; not used anywhere yet |
| AU manufacturer and subtype codes | not chosen | decide when the AU target starts (4 characters each; register with Apple) |

Rules:

- The spike ids are separate from the product ids on purpose: the spike must never be mistaken for, or collide with, the real instrument in a host's plugin list or in a saved project.
- A second plugin from the project (an effect, a standalone) gets its own CLAP id and class id; never reuse one.
- Host parameter ids are permanent too, for the same reason. The shell spike's are the strings `macro_1`…`macro_8` and `slot_1`…`slot_64`; nice-plug hashes them to the 31-bit ids hosts see (`macro_1` is `825615486`, `slot_1` is `1248030256`, `slot_64` is `34232479`). The patch blob lives under the state field `patch`. The real synth keeps this scheme. `cargo test -p spike-shell` and `cargo xtask reaper-test` fail if any of them change.
- Changing the product name later means changing the display name and the crate prefix only.
