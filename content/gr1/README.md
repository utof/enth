# content/gr1

Data for gate GR1 ([docs/gr1.md](../../docs/gr1.md)), made by R1/S1 lane INT on 2026-10-20 with this build's generator v1 and scorers (`sound/1`, `structure/1`, `combined/1`).

| File | What it is |
| --- | --- |
| `codes.txt` | The 20 codes everyone plays: 4 per level, `enth challenge new --level N --seed S` with S = 1001 to 1004. Short codes, so they carry no target-hash prefix; the round log records each target's fingerprint, and `gr1-report` flags a code with two fingerprints |
| `floors.jsonl` | The two floors of the agent baseline on those codes, as `enth.gr1-baseline/1` lines: `noop` (the untouched start patch) and `cmaes` (`enth match --target <code> --free observable --budget 1500 --workers 4`: CMA-ES over every observable parameter from the challenge's start, 1,500 renders; the result is the same for any worker count). Pass it to the report with `--baseline content/gr1/floors.jsonl` |

The floors: the no-op start passes 0 of 20 (sound 3.6 to 43.4 %); CMA-ES passes 13 of 20 (levels 1 to 4: 3 of 4 each; level 5: 1 of 4, seed 1001 at 90.18 % against the 90 % mark; the rest stay far below it: structure is never searched). Re-made on engine 0.3.0 (FU1 lane DRV, the filter drive's make-up gain) with INT's script; on 0.2.0 CMA-ES passed 12 of 20 (level 5 seed 1001 at 89.93 %), and five other lines moved by at most 0.15 sound %. Seed 1004 fails at every level (sound about 60 %), probably on its oscillator draws, which keyed streams share across levels (not investigated).

The agent arm (three attempts per code, a paid run) goes in its own file beside these when you run it ([docs/gr1.md](../../docs/gr1.md#making-the-baseline-lines)). Re-make the floors after a generator or scorer change: the lines carry the scores of this build only.
