# Spike: CLAP test host (Phase 0, lane H)

- Date: 2026-10-19 (Phase 0 lane H); extended the same day by P1b lane TH (branch `p1b/th`)
- Code: `tools/test-host` (package `enth-test-host`), `tools/test-host/proof.sh`, `xtask/src/host_render.rs`, CI job `host-render`. Branch `lane-h/test-host`, then `p1b/th`.
- Decision it feeds: how we test the plugin as a plugin, without REAPER. Companion to [shell.md](shell.md) (the REAPER tests).
- Verdict: **the host works and answers the three questions it was built for.**
  1. A plugin-level golden render is cheap (25 ms for 2 s of audio) and bit-stable: debug and release bundles give the same hashes.
  2. The researcher's REAPER observation (a second spike-shell instance renders differently) **does not reproduce** here, in four different driving modes. The plugin has no cross-instance state that changes the sound. The likely cause is on the host side (below), not proven.
  3. The editor open/close loop **leaks about 77 KB per cycle** on spike-shell (Xvfb, Mesa software GL), threads do not leak, and a baseline without the editor leaks 0.2 KB per cycle. About 61 KB of the 77 is one missing `xkb_keymap_unref` in baseview 0.4.1. With that one line patched in a scratch copy the growth is 16 KB per cycle.

**P1b additions (lane TH).** The host also loads a patch document as state, replays the engine's golden cases through a plugin, churns instances, floods the event input, checks `NOTE_END`, measures `process()` on a real-time thread and times a parameter event to its audible effect. Each of the seven has a way to make it fail on purpose, and `tools/test-host/proof.sh` runs all of them with those controls on spike-shell: [P1b modes](#p1b-modes-lane-th), [Proof on spike-shell](#proof-on-spike-shell).

Everything with an editor ran under Xvfb with Mesa software GL (llvmpipe), never on the real display. Numbers from that setup are labelled "Xvfb" below and say nothing about a real GPU driver.

## What was built

A minimal CLAP host on `clack-host =0.2.0` (MIT or Apache-2.0, current release 2026-09-12; `clack-extensions =0.2.0` for the extension bindings). About 3,300 lines including tests in Phase 0, about 9,800 after P1b (the seven modes, their tests and the Stub plugin). It is deliberately not a general host: it loads one `.clap`, drives it the way the tests need and prints JSON.

- Host extensions offered: `log`, `gui` (resize and closed notifications), `params` (rescan and flush requests), `state` (dirty). No `thread-check`; nice-plug then compares thread ids itself.
- Plugin extensions used: `audio-ports`, `note-ports`, `params`, `state`, `gui` (embedded X11 only, which is all nice-plug supports).
- `process()` runs on one thread. By default that is a dedicated thread, because nice-plug runs GUI work inline when `process()` happens to be on the main thread (see Limits).

## Usage

```
cargo build -p enth-test-host            # target/debug/enth-test-host
cargo xtask bundle spike-shell [--release]
enth-test-host info <plugin.clap>
enth-test-host render <plugin.clap> --notes "C4@0+1.5:0.8, E4@0+1.5, G4@0+1.5" --sr 48000 --block 64 --dur 2 \
    [--state blob] [--param "Macro 1=0.3"]... [--out out.wav] [--save-state FILE] [--process-on thread|caller]
enth-test-host instances <plugin.clap> --notes ... -n 2 [--modes sequential-fresh,sequential-late,lockstep,concurrent]
xvfb-run -a -s "-screen 0 1920x1080x24" \
    enth-test-host editor-cycles <plugin.clap> -n 200 [--mode both|editor|baseline] [--repeat 2] [--json-out FILE]
# P1b (lane TH), each described under "P1b modes":
enth-test-host render|instances|... --state patch.json [--state-format auto|blob|patch|envelope] [--state-field patch]
enth-test-host cli-equal <plugin.clap> [--cases DIR] [--expected DIR | --no-expected] [--profile FILE] [--blocks 1,64,100,4096]
enth-test-host churn <plugin.clap> [-n 200] [--max-kb-per-cycle 1]
enth-test-host bursts <plugin.clap> [--counts 16,64,256,1024,4096] [--capacity N] [--order both|notes-first|params-first]
enth-test-host note-end <plugin.clap> [--notes SCRIPT] [--blocks 1,32,64,100,512]
enth-test-host process-stats <plugin.clap> [--gate g1|g1-multi] [--secs 120] [--rt 10|none] [--instances N --threads T] [--editor]
enth-test-host latency <plugin.clap> --param "Macro 1" --from 1.0 --to 0.2 [--max-onset-ms N]
tools/test-host/proof.sh                 # every mode and its negative control on spike-shell
```

Every command prints one JSON document on stdout (pretty only on a terminal). Schemas: `enth.test-host.info/1`, `.render/1`, `.instances/1`, `.editor-cycles/1` (the combined report), `.editor-run/1` (one process) and, from P1b, `.cli-equal/1`, `.churn/1`, `.bursts/1`, `.note-end/1`, `.process-stats/1` and `.latency/1`. Errors are JSON on stderr, schema `enth.test-host.error/1`. Exit codes: 0 ok, 1 a check failed (`instances` found a difference, a hash differs, a budget is exceeded), 2 error. Run `enth-test-host --help` for the full option list.

**xvfb-run folds the command's stderr into its stdout**, so plugin log lines and Mesa warnings (`libEGL warning: DRI3 ...`) land in the JSON. Pass `--json-out FILE` (any command) and read the file. `editor-cycles --mode both` also captures each child's stderr separately.

### `info`

Descriptor (id, name, vendor, features), every parameter (id, index, name, module, flags, min, max, default, current value), audio ports (id, channels, type) and note ports (dialects), plus the list of extensions the plugin exposes. For spike-shell: 72 parameters, ids as in [shell.md](shell.md) (`Macro 1` is 825615486, `Slot 1` is 1248030256), one stereo output, one note input.

### `render`

Offline and deterministic by construction:

- Fixed block size. The frame count is `ceil(dur * sr / block) * block`, so the last block is full.
- Events sit at exact sample positions inside the block that contains them. At one position they are ordered reset, parameter change, note-off, note-on.
- The hash is SHA-256 over the interleaved little-endian f32 samples of every output channel. The WAV (32-bit float, with a `fact` chunk) is written only when `--out` is given. The report also has peak, RMS, a count of non-finite samples and the render time.
- `--state` loads a state blob through the `state` extension before the first block; `--param NAME=VALUE` sets a parameter (name or numeric id) before the first block; `--save-state` writes the state back out after the render.

The notes script: entries separated by `,`, `;` or a newline, `#` starts a comment, a file path is read as a script.

| Entry | Meaning |
| --- | --- |
| `C4@0+1.5:0.8` | key `@` start `+` duration `:` velocity (velocity optional, default 1.0); seconds. Keys are MIDI numbers or names, C4 = 60 |
| `Slot 1=0.2@0.5` | set a parameter (name or numeric id) to a plain value at 0.5 s |
| `reset@0.5` | the host calls the plugin's `reset()` at 0.5 s (held voices end) |

### `instances`

Renders the same script through N instances in one process and compares each instance's hash with a reference from a lone instance. Four driving modes, because a shared-state bug may only show under one of them:

| Mode | What it does |
| --- | --- |
| `sequential-fresh` | instance 1 is created, rendered and destroyed, then instance 2 is created and rendered, and so on |
| `sequential-late` | all N are created and activated first, then rendered one after another |
| `lockstep` | all N are processed block by block, interleaved, on one thread |
| `concurrent` | all N are processed at the same time on N threads |

The report has the per-instance hashes, peaks and RMS per mode and `identical`. Exit code 1 if any hash differs from the reference. The in-process test plugin in `tools/test-host/tests/host.rs` has a deliberately leaky variant (a static counter changes its gain) and the command catches it, so a pass means something.

### `editor-cycles`

Opens and closes the editor N times and records, after each close: PSS (from `/proc/self/smaps_rollup`, in KB, plus the anonymous part), the thread count (open and closed), the open time, and whether the first frame was drawn.

- **Only under Xvfb.** The command refuses to run unless `WAYLAND_DISPLAY` is unset, `DISPLAY` is a local display and a process named `Xvfb` was started with that display as an argument (it scans `/proc/*/cmdline`; `xvfb-run` starts it exactly that way). The check runs before the plugin library is loaded and before any X connection. A real Xorg, Xwayland, a remote `DISPLAY`, or no `DISPLAY` is refused with a hint.
- The host creates the parent window itself (pure-Rust `x11rb`), paints it magenta, calls `create`, `set_scale`, `get_size`, `set_parent`, `show`, holds the window for `--hold-ms` (default 100) and then `hide` and `destroy`, with `--settle-ms` (default 50) before sampling.
- "First frame drawn" means a pixel of the parent window (read back with GetImage in strips) is no longer magenta within `--first-frame-timeout-ms` (default 1000). `blank_before_show` is checked in every cycle first, to prove the probe can tell a blank window from a drawn one; the distinct colour count of the final full-window read (175 for spike-shell) shows it is not a flat fill.
- An audio thread processes a held chord, paced in real time, during the whole run, so the cycle includes a live `process()` as in a host.
- `--mode both` (default) runs the baseline (the same host loop, same audio thread, no editor) and the editor run as separate child processes, `--repeat` times each, so memory never mixes. `--mode editor|baseline` runs one in the current process: that is the form to put under a profiler.
- The summary has the least-squares slope of PSS over the cycles after `--warmup` (default 10) and over their second half, the total growth, thread counts, open-time min/median/p95/max, the number of opens drawn in time, thread-name maps (before, open in the first cycle, closed in the last, after shutdown), the number of `gui.closed` notifications and resize requests, create failures and the plugin's log lines. `--mode both` adds `editor_minus_baseline_kb_per_cycle` and the count of `X11 thread disconnected` lines from the children's stderr.

## Golden render in CI

`cargo xtask host-render [--release] [--no-build] [--update]` (Linux only) bundles spike-shell, builds the test host, renders the seven cases in `tools/test-host/golden/spike-shell.toml` with `DISPLAY` and `WAYLAND_DISPLAY` removed from the environment (48 kHz, block 64, 2 s) and compares each hash with the committed one. It also fails on non-finite samples and on silence, because a golden of silence proves nothing. WAVs go to `target/host-render/`.

| Case | What it covers | SHA-256 of the samples (first 16 hex) |
| --- | --- | --- |
| chord | three held notes, defaults | `1fd6c79cfdded55b` |
| automation | overlapping notes, host automation of Macro 1 and Slot 1 | `9cf94691af319d36` |
| voice-steal | ten notes for eight voices | `54d85267f4e7739e` |
| state-restore | a state blob (macro 1 = 0.9, slot 1 = 0.1) loaded before activation | `d340047cfd33f6a7` |
| state-patch | a patch document (`golden/spike-shell.patch.json`, auto-detected by `--state`) wrapped into the persisted `patch` field; the patch blob is inert, so the sound is the chord's | `1fd6c79cfdded55b` |
| cli-params | `--param` before the first block | `9b5805f4a868b5af` |
| reset | `reset()` mid-note, a later note still sounds | `71cad96ded47fd7e` |

Debug and release bundles gave the same six hashes in Phase 0, on this machine; `state-patch` was checked on the release bundle only (its hash is the chord's, and the debug bundle gives that one). A changed hash means the sound, the parameter or state handling, or the host changed. If that is on purpose: `cargo xtask host-render --update` (it rewrites only the `sha256` lines) and say why in the commit message.

The CI job `host-render` (ubuntu-24.04, no display) runs `cargo xtask host-render` and then `enth-test-host instances` with three instances. `actionlint` 1.7.12 is clean.

## P1b modes (lane TH)

Seven additions, each with a way to make it fail on purpose. A check that cannot fail proves nothing, so every mode below has a **negative control** (a host-side `--inject` fault, a state that makes the in-process test plugin misbehave, or a budget no plugin can meet), and [Proof on spike-shell](#proof-on-spike-shell) runs each control and asserts that it fails for the reason it was made for.

Shared conventions: every command takes `--json-out FILE`, prints one JSON report (schema `enth.test-host.<command>/1`) and exits 0 when its checks hold, 1 when one fails and 2 on an error (bad input, a plugin that cannot load). `DISPLAY` and `WAYLAND_DISPLAY` are never read except by the two editor paths, which refuse to run unless `DISPLAY` is an Xvfb server. Nothing here plays audio.

### 1. `--state FILE`: load a patch into the plugin

`--state FILE [--state-format auto|blob|patch|envelope] [--state-field patch]` is accepted by `render`, `instances`, `churn`, `bursts`, `note-end`, `process-stats` and `latency` (`cli-equal` loads the engine's patches through the same path). The state goes through the CLAP state extension before activation, as a host loading a project does.

| `FILE` is | Auto-detected when | What the host does |
| --- | --- | --- |
| a **blob** | the text is not JSON (what `render --save-state` writes) | passes it through unchanged |
| a **patch document** | JSON that is not an envelope (`golden/spike-shell.patch.json`, any `*.patch.json`) | puts the text into `fields.<--state-field>` of an otherwise empty envelope, so every parameter keeps its default, and adds the length prefix |
| an **envelope** | one JSON object with `params` and `fields` members (a decoded blob) | adds only the length prefix |

The nice-plug CLAP state is `u64` little-endian length plus the JSON `{"version","params":{id:{"f32":v}},"fields":{name:"<JSON text>"}}`. spike-shell keeps its patch in the persisted field `patch`. [design.md section 4](../design.md) says `enth-plugin` keeps the exact `Patch::to_json()` text in the same field, "a plugin state and a `.patch.json` are interchangeable", so the host needs no `enth-plugin` code: `--state some.patch.json` is the whole interface. Wrapping needs nothing but `serde_json`.

`render --check-roundtrip` reports `state.patch_round_trip`: whether the plugin's saved state has the field (`saved_field_present`), whether its text is byte-identical to the file (`identical_text`) and whether the JSON is equal (`equal_json`). spike-shell parses the document and writes it back with its keys sorted (the 138-byte fixture, with its trailing newline, comes back as 137 bytes), so `identical_text` is false and `equal_json` true. For `enth-plugin`, where the field is the exact `Patch::to_json()` text, both should be true for a file written without a trailing newline; a trailing newline in a `.patch.json` makes `identical_text` false even then, which is the file's doing, not the plugin's.

Negative controls: `--state-field not-a-field` (the plugin does not know the field, so `saved_field_present` is false and the run exits 1), a file that is not JSON with `--state-format patch` (exit 2).

A new golden case, `state-patch`, loads `golden/spike-shell.patch.json` this way; its hash equals the plain chord's because spike-shell's patch blob is inert (it is not sound-bearing), which is the point of the case: the state path is exercised by `cargo xtask host-render` in CI.

### 2. `cli-equal`: the engine's golden cases through the plugin

```
enth-test-host cli-equal <plugin.clap> [--cases DIR|FILE]... [--expected DIR | --no-expected] [--only NAME]...
    [--blocks 1,64,100,4096] [--profile FILE] [--state-field patch] [--min-compared 1]
    [--write-expected DIR] [--inject flip-bit]
```

Defaults are `tests/golden/cases` and `tests/golden/expected`, found by walking up from the current directory. For each `*.toml` case: the `[source.patch]` table becomes JSON and goes into the plugin's `patch` field through the state path, the notes become CLAP note events and the automation host parameter events at `round(t * sample_rate)` frames (same-frame order as the engine: automation, then each note's on and off in performance order; the note index is the note id), and the case is rendered for `round(seconds * sample_rate)` frames at **each** host block size (default 1, 64, 100 and 4096: 1 and 100 are where block-quantised event handling shows). The hash is the engine's: SHA-256 over the sample rate (`u32`), the channel count (`u16`) and the sample count (`u64`), all little-endian, then every `f32` bit pattern, channels interleaved (`hash_samples` in `tests/golden/src/summary.rs`).

Outcome per case: **equal** (all block sizes agree and match the engine), **block-dependent** (block sizes disagree: a plugin bug whatever the engine says), **differs** (block sizes agree on a hash that is not the engine's), **skipped with its reason** or **error**. The run fails on any block-dependent, differs or error case, and when fewer than `--min-compared` cases were compared (default 1, so a run in which everything was skipped cannot pass).

What cannot go through a plugin is skipped, never silently:

- sources that are not a patch (the harness's `sine`), and render seeds other than the default `0x0E17_5EED` (the plugin has no seed input);
- automation of engine addresses (`osc1.level`, `filter1.cutoff`, ...). The plugin's host parameters are macros and slots, so `--profile FILE` can supply host parameter events for a case instead.

The profile is TOML (the host reads it with its own small parser, see Limits); every key is optional:

```toml
state_field = "patch"                 # the persisted field the patch goes in
blocks = [1, 64, 100, 4096]           # host block sizes
cases = ["osc-saw-440", "adsr-pad"]   # only these cases
[automation]                          # host events standing in for a case's engine-address automation
svf-cutoff-jump = "Slot 1=0.3@0.4003, Slot 2=0.9@0.5"
```

`--write-expected DIR` writes the plugin's own hashes in the engine's `expected/*.toml` shape; `--no-expected` checks block independence only.

Negative controls: `--inject flip-bit` flips one bit of the first sample before hashing (every case must now differ, so the comparison can fail), and the engine's real hashes against a plugin that renders something else (spike-shell does not render engine patches) must differ.

For `enth-plugin` the expectation is: the 62 cases that reach a plugin all **equal**; the 10 that cannot are the 4 seeded/stand-in cases (`engine-chord`, `lfo-seeded-sh`, `voice-mono`, `stand-in-sine`) plus 6 with engine-address automation (`comp-steps`, `mod-routes`, `osc-wave-switch`, `reverb-params`, `reverb-wake`, `svf-cutoff-jump`) that need a profile. Recommendation: raise `--min-compared` from its default of 1 to 60 in the real gate, so a profile that skips more than expected cannot pass.

### 3. `churn`: instance lifetimes

```
enth-test-host churn <plugin.clap> [-n 200] [--warmup 10] [--notes SCRIPT] [--state FILE ...] [--block 64] [--blocks 64]
    [--settle-ms 10] [--max-kb-per-cycle 1] [--max-thread-growth 0] [--max-fd-growth 0]
    [--process-on thread|caller] [--inject leak-kb=N|fd|thread]
```

One cycle is a whole life: instantiate, load state, activate, process `--blocks` blocks of a chord that is released half-way (so note-off and release paths run), deactivate, drop. After `--settle-ms` it records PSS (`/proc/self/smaps_rollup`, so shared libraries count once), the thread count, the open descriptors and the mapping count. The first `--warmup` cycles are ignored. The run fails when the least-squares PSS slope over the rest, or over its second half, exceeds `--max-kb-per-cycle`, or when threads or descriptors grew by more than the limits (default 0: a thread or descriptor left behind by a cycle fails). Each cycle's audio runs on a fresh thread by default (`--process-on thread`; `caller` runs it on the calling thread).

Sensitivity, measured with `--inject leak-kb=N`: 16 KB per instance gives a slope of 16.1 KB per cycle and fails; 2 KB per instance is detected; **1 KB per instance is not** at the 1.0 KB threshold (the slope is noise at that size). Do not read a pass as "no leak below 1 KB". `--inject fd` and `--inject thread` make the host leak a descriptor or a parked thread per cycle; each fails its own measure.

The in-process Stub plugin in the tests also leaks on request (`{"leak":"mem:32"}`, `"fd"`, `"thread"` in its state), which proves the checks see leaks made inside a plugin, not only the host's own.

### 4. `bursts`: event floods and `INPUT_EVENT_CAPACITY`

```
enth-test-host bursts <plugin.clap> [--counts 16,64,256,1024,4096] [--kind mixed|notes|params] [--block 64]
    [--capacity N] [--order both|notes-first|params-first] [--warm-blocks 8] [--after-blocks 8] [--tail 1.0]
    [--allow-unended] [--inject drop-after=K]
```

For each count N, one fresh instance plays a held chord for a few blocks, then **one block** holding N events (note-on/off pairs and parameter changes, spread over the block so several share a frame), then ordinary blocks, a probe note and a tail. nice-plug issue 87: the frame count does not bound the event count, and a plugin that copies events into a fixed queue (`INPUT_EVENT_CAPACITY`) must do something defined when it is full. Checks per trial: no `process()` error (a plugin that aborts kills the host and fails with the signal), finite samples, **deterministic** (a second fresh instance gives the same samples and `NOTE_END` list), **recovered** (the probe note after the burst sounds and ends), **`NOTE_END` complete** (see 5), and **applied**: the same events fed one frame per block, where no queue overflows, give the same samples; if they do the burst applied every event, otherwise some were dropped.

`--capacity C` declares the plugin's queue size: every N <= C must be fully applied (a failure otherwise), and a larger N that is dropped is reported (`applied_all_up_to_events`) and is **not** a failure, because dropping at overflow is the documented behaviour. The report also has the burst block's `process()` time next to a typical block's. `--order` is the order of events that share a frame, both by default, since it matters (see Findings).

Negative control: `--inject drop-after=K` delivers only K events of the block, as a plugin with a K-event queue would see them; with `--capacity` above K it fails ("not all applied", and the dropped note-offs leave notes unended).

### 5. `note-end`: every note ends, inside its block

```
enth-test-host note-end <plugin.clap> [--notes SCRIPT] [--blocks 1,32,64,100,512] [--tail 2] [--state FILE ...]
    [--allow-unended] [--inject drop-end=K,late-end=F]
```

Plays a notes script at each block size, with a tail, and checks every `NOTE_END` the plugin sent (`events::check_note_ends`, also used by `bursts`): every note the host started ended **exactly once**, with the key (and note id) of its note-on, **inside the block** it was sent in (offset below the block's frame count: CLAP says the offset is relative to the block), after its note-on (no premature end) and in time order within the block; no `NOTE_END` for a note that never started. The same set of notes must end at every block size. The default script has 15 notes: chords, more notes than the 8 voices (steals), a duplicate key, a near-zero-length note (`D4@0.25+0.00001`; a zero-length note is rejected by the script parser) and notes that end between blocks.

Negative controls: `--inject drop-end=3` hides every third `NOTE_END` (5 of 15 notes unended) and `--inject late-end=70` moves each 70 frames later, past the end of a 64-frame block (15 `NOTE_END`s outside their block).

### 6. `process-stats`: `process()` timing on real-time threads

```
enth-test-host process-stats <plugin.clap> [--secs 120] [--warmup 10] [--block 64] [--sr 48000] [--rt 10|none] [--require-rt]
    [--pin CPU,CPU] [--instances 1] [--threads 1] [--editor] [--notes SCRIPT] [--state FILE ...]
    [--gate g1|g1-multi] [--max-p99-us N] [--max-p999-us N] [--max-callback-p99-pct N] [--inject busy-us=N[,every=K]]
```

The G1 row for `process()` in [benchmarks.md](../benchmarks.md#real-time-measurement-p1b), driven the way a host drives it:

- **Paced.** Each audio thread wakes every `block / sample_rate` seconds on an absolute `CLOCK_MONOTONIC` timer (`clock_nanosleep` with `TIMER_ABSTIME`), then processes its instances. Sleeping lets caches go cold and cores clock down, which an unpaced loop hides.
- **Real-time priority.** The default is `SCHED_RR` 10, got from rtkit: the command lowers `RLIMIT_RTTIME` to 200 ms (rtkit's condition), calls `org.freedesktop.RealtimeKit1.MakeThreadRealtimeWithPID` through `busctl`, and reads the policy back with `sched_getscheduler` rather than trust the reply. No sudo, no `ulimit -r`. A real-time thread that never sleeps is killed by the limit instead of freezing the desktop; this loop sleeps every period and resynchronises after falling 10 periods behind. rtkit allows 25 real-time threads per user and PipeWire holds four, so use few. `--rt none` measures at normal priority (the report says so and warns); `--require-rt` turns a refused promotion into a failure, which `--gate` implies.
- **Warm-up.** The first `--warmup` seconds are not recorded.
- **What is recorded**, per thread and for the plugin alone: the wall time of the CLAP `process()` call (p50, p90, p99, p99.9, p99.99, max, mean and a log2 histogram from 1 to 65,536 microseconds), the callback around it (the host's event building plus every instance of the thread; wall and thread CPU time), the wake-up lateness, deadline misses (callbacks that ended after the next period began) and resynchronisations. The shares of the period (1333 microseconds at 64 frames and 48 kHz) are in the report.
- **`--instances N --threads T`**: N instances spread over T threads, each thread with its own timer. 16 instances on one thread cannot keep up on this laptop, so `g1-multi` uses 4 threads.
- **`--editor`**: opens the plugin's editor in a host-made X11 window for the whole run (only under Xvfb), for an A/B against a run without.
- **`environment`**: `uptime`, load average at the start and end, kernel and preemption model, CPU model, governor, energy preference and frequency, PipeWire's quantum and rate, rtkit's limits, `RLIMIT_RTPRIO` and `RLIMIT_RTTIME`, available memory and swap use. The numbers mean little without it.

Budgets: `--gate g1` is p99 and p99.9 of `process()` below 333 microseconds (25 percent of the period) with `--require-rt`; `--gate g1-multi` is 16 instances on 4 threads with each thread's callback p99 below 50 percent of the period (the information gate in the roadmap), with `--require-rt`. `--max-p99-us`, `--max-p999-us` and `--max-callback-p99-pct` set them by hand. A gate run over a 120 s window takes 130 s of wall clock. Run it under `flock -x /tmp/enth-quiet.lock` so nothing builds meanwhile.

Negative controls: `--inject busy-us=400,every=200` makes every 200th `process()` spin for 400 microseconds inside the timed window (p99.9 jumps to 412, the p99.9 budget fails and p99 does not move), `--rt 99 --require-rt` is refused by rtkit (max priority 20) and fails, and `--max-p99-us 1` cannot be met.

### 7. `latency`: knob to sound

```
enth-test-host latency <plugin.clap> --param NAME --to VALUE [--from VALUE] [--notes SCRIPT] [--at 0.5]
    [--offsets 0,17,63] [--after 0.5] [--block 64] [--threshold-db -60] [--max-onset-ms N] [--max-audible-ms N]
    [--inject delay-blocks=K]
```

A differential measurement, so no microphone is needed. Two fresh instances play the same held notes; one also gets the parameter event at an exact frame `F` (at `--offsets` positions inside its block). The outputs must be bit-identical before `F` (else the plugin is not deterministic and the number would be meaningless); after it `delta = with - without`. Reported, in frames and milliseconds:

- **onset**: the first frame at or after `F` where the outputs differ at all, the plugin's event latency. 0 for a sample-accurate plugin, up to a block for one that applies parameters at block starts. If the onset is the same at every offset the report says `sample_accurate`; if it is `block - offset` it says `events_land_on_a_grid`.
- **audible**: the first frame where the trailing 1 ms peak of `|delta|` reaches `--threshold-db` (default -60 dBFS).
- **settled**: the first frame where that envelope reaches 90 percent of its final level (the smoother's time).
- `process_us`: the wall time of the block that carried the event.

`--max-onset-ms` and `--max-audible-ms` fail the run when exceeded. The measurement ends at the event in the plugin: the path from a GUI knob to that event (the editor's own latency, the host's parameter flush) is not in it.

Negative controls: a parameter that does nothing (`"Macro 2"` on spike-shell) ends with "no audible change" and exit 1, and `--inject delay-blocks=3` delivers the event three blocks late while the measurement still expects it on time, so the onset reads 192 frames (4 ms) and trips `--max-onset-ms 1`.

### Tests

`cargo test -p enth-test-host` has 43 unit tests and, for the integration tests, an in-process **Stub** plugin (`tests/common/mod.rs`, built on `clack-plugin` and registered through `PluginEntry::load_from_clack`, so no bundle is needed) that does what the patch loaded through the state extension asks it to: leak memory, a descriptor or a thread per instance (`leak`), end notes wrongly, twice, early, outside their block or never (`note_end`), keep at most `cap` events of a block and then drop them, wedge or write NaN (`policy`), apply a parameter exactly, at the next block start or N frames late (`param_mode`), spin inside `process()` (`busy_us`), behave differently per instance (`nondeterministic`), or render a signal the test can compute itself (`mode`). `tests/modes.rs` (23 tests) runs every mode against the Stub both clean (must pass) and faulty (must fail with the right message); `tests/churn.rs` runs the churn scenarios in sequence (it counts the process's threads and descriptors, so nothing else may run beside it).

## Results

### `instances`: the REAPER observation does not reproduce

Research 06 saw a second spike-shell instance in the same REAPER process render at RMS 0.086 to 0.096 where the first gave 0.123, while first-instance renders were bit-identical across processes.

Here, with N = 2 and N = 3, on the debug and the release bundle, all four modes give the reference hash `1fd6c79c...` (the chord case; RMS 0.0595, peak 0.2826) for every instance. Exit code 0.

Reading spike-shell for shared state: `lib.rs` and `synth.rs` have no mutable `static`, `thread_local!`, lazy global or custom allocator (the only `static` items are the `&'static` trait constants). The one time-dependent thing is the nonce written into the patch blob on the first `activate`, and it is not audible. Each instance owns its voices, smoothers and parameters.

What differs in REAPER is the host. [shell.md](shell.md) records that REAPER's main thread deactivates, reactivates and resets each CLAP plugin about 30 ms into an offline render, and that this drops a held note. The test host shows how much a reset changes a render. Take two held notes (C4, G4 for 1.5 s) and a later E4, 2 s in all: without a reset the RMS is 0.0530; with `reset@0.5` it is 0.0353; with `reset@0.03`, the timing REAPER uses, it is 0.0223 because the held notes are gone. The REAPER numbers also varied from run to run (0.086 to 0.096 against 0.123). The 06 script rendered, deleted and re-added the instance and rendered again in the same process; the second render can see a different restart timing. **Hypothesis, not proven:** the difference came from when REAPER's restart landed relative to the note, not from state in the plugin. A cross-check in REAPER (render the same project several times in one process, log the plugin's `activate`/`reset` calls with timestamps) would settle it. It was not done here: REAPER only runs under Wine on this machine and RAM is tight.

### `editor-cycles` on spike-shell (Xvfb, Mesa software GL)

Setup: release bundle, `xvfb-run -a -s "-screen 0 1920x1080x24"`, 200 cycles, `--mode both --repeat 2`, hold 100 ms, settle 50 ms, 10 warm-up cycles, block 256, window 360 x 200, run under the exclusive build lock (the other lane's locked builds were not running, but the machine was not idle). Load average 5.96 at the start and 3.22 at the end (the child processes saw 3.2 to 4.0). Kernel 7.0.0-38, Mesa software rendering (llvmpipe); the real GPU was not touched. Total 3 min 12 s. The raw reports are in the lane's scratch directory (`m1-release-200x2.json`).

| Measure | Baseline (no editor) | Editor, as shipped |
| --- | --- | --- |
| PSS growth per cycle, after warm-up | 0.20 KB (both repeats) | **76.7 and 77.5 KB** |
| Same, second half of the run | 0.20 KB | 87.5 and 78.7 KB |
| PSS at cycle 1 and cycle 200 | 3.2 MB to 3.25 MB | 68.4 MB to 113.6 MB, and 68.5 MB to 109.6 MB |
| Editor minus baseline | | **76.9 KB per cycle** |
| Threads, editor closed | 3, constant over 200 cycles | 3, constant over 200 cycles |
| Threads, editor open | 3 | 30, constant (12 `llvmpipe-N`, the plugin's window and audio threads, and others) |
| Open time (create to show) | | median 53.1 ms, p95 69 ms, min 46 ms, max 87 ms |
| First frame drawn within 1 s | | 200 of 200 (both repeats); `blank_before_show` true every cycle |
| `X11 thread disconnected` lines | 0 | 0 |
| `gui.closed` notifications, resize requests, create failures | 0 | 0 |

How to read it:

- The first three cycles add about 26 MB (Mesa, llvmpipe and the GL set-up load once), so the total growth of 41 to 45 MB over 200 cycles is mostly not leak. That is why the slope is taken after the warm-up and compared with the baseline.
- The leak is real and steady: about 77 KB per cycle, 15 MB per 200 cycles. The two repeats agree within 1 KB per cycle; the baseline is flat to 0.2 KB.
- There is no thread leak: 3 threads after every close, 30 while open, in all 200 cycles.
- For scale, research 02 measured (in REAPER, different harness) about 15 KB per cycle for ReaEQ, about 70 KB for a softbuffer baseview editor and 115 to 160 KB for egui with glow, with the thread count returning to 27. The numbers are not directly comparable (different host, driver and window), but the order of magnitude is the same, and here the cause is below egui.

### Where the 77 KB comes from

Not measured with heaptrack: it segfaults as soon as the plugin opens a window (an endless recursion between glibc's `_dl_update_slotinfo` and heaptrack's `free` hook, on the first thread created after the plugin was loaded). It works in `--mode baseline`. `valgrind --leak-check=full` does work on the editor mode (about 100 s for 6 cycles; its PSS figures are meaningless, only the leak records count):

```
xvfb-run -a -s "-screen 0 1920x1080x24" valgrind --leak-check=full --show-leak-kinds=definite --num-callers=14 \
  enth-test-host editor-cycles plugin.clap -n 6 --mode editor --warmup 2 --hold-ms 300 --settle-ms 100 --first-frame-timeout-ms 5000
```

Over 6 cycles: 342 KB lost (definitely plus indirectly), which is 57 KB per cycle.

| Source | Per cycle | Where |
| --- | --- | --- |
| xkb keymap | **53.6 KB** (321,882 B in 6 blocks) | `xkb_x11_keymap_new_from_device` called from `baseview::platform::x11::event_loop::EventLoop::new` |
| EGL display | about 3.4 KB (20.7 KB in 7 records) | Mesa allocations made under `baseview::wrappers::egl::Egl::create_display` (and three small Mesa/DRI records with no symbols) that `eglTerminate` does not free |

The cause is in baseview 0.4.1, `src/wrappers/xkbcommon.rs`. `XkbcommonState::new` creates a keymap (reference count 1), hands it to `xkb_x11_state_new_from_device` (which takes its own reference) and never calls `xkb_keymap_unref` on its own. The comment says "keymap should be unref when XkbcommonState drop", but `Drop` unrefs only the state. Each editor window therefore leaks one keymap; its size depends on the keyboard layout (here the default Xvfb layout).

**Check by experiment** (scratch copy of baseview with `xkb_keymap_unref(keymap)` added after the state is created, built through `cargo build --release -p spike-shell --config 'patch.crates-io.baseview.path=...'`; not committed, the worktree's `Cargo.lock` is unchanged): same setup, 200 cycles, `--repeat 2`, load 5.73 at the start and 4.65 at the end.

| | As shipped | With the one-line patch |
| --- | --- | --- |
| Editor PSS growth per cycle, after warm-up | 76.7 and 77.5 KB | **15.9 and 16.9 KB** |
| Second half | 87.5 and 78.7 KB | 8.2 and 8.2 KB |
| Editor minus baseline | 76.9 KB | 16.1 KB |
| Baseline | 0.20 KB | 0.22 KB |
| Open time (median), drawn in time | 53.1 ms, 200/200 | 53.6 ms, 200/200 |

So the patch removes 61 KB of the 77 (valgrind accounts for 54; the rest is allocator behaviour around those blocks). The remaining 16 KB per cycle (8 KB in the second half) is probably the EGL part plus allocator noise; it is not attributed. The fix belongs upstream (baseview or nice-plug's fork); we should not carry it until we decide how we depend on baseview.

## Proof on spike-shell

`tools/test-host/proof.sh` runs every mode on a plugin (spike-shell by default) together with its negative control and compares each command's exit status with the one wanted; `why` lines then check that a failing run failed for the reason its fault was injected for (a control that fails for some other reason proves nothing). The exit status of the script is the verdict. The 45 checks (30 commands, of which 14 must pass, 15 must fail and 1 must be an error, and 15 `why` assertions) all hold; `--no-timing` and `--no-editor` skip some. It builds nothing, so first `cargo xtask bundle spike-shell --release` and `cargo build --release -p enth-test-host`. Reports go to `target/test-host-proof/`.

| Mode | Command (shortened) | Exit | Why it matters |
| --- | --- | --- | --- |
| `--state` | `render --state golden/spike-shell.patch.json --check-roundtrip` | 0 | the patch reaches the plugin's `patch` field and back (`equal_json` true) |
| | the same with `--state-field not-a-field` | 1 | `saved_field_present` is false: a wrong field is noticed |
| | `--state` a text file with `--state-format patch` | 2 | not JSON is an error, not a silent empty state |
| | `--state` a saved blob | 0 | the old form still works |
| `cli-equal` | `cli-equal --no-expected` | 0 | 62 compared, 62 block-independent, 10 skipped with reasons |
| | `--write-expected DIR`, then `--expected DIR` | 0 / 0 | the mechanism: hashes the plugin wrote equal the hashes it gives (62 of 62) |
| | `--expected DIR --inject flip-bit` | 1 | negative control: 62 of 62 differ, none equal |
| | the engine's real `tests/golden/expected` | 1 | spike-shell is not the engine: 62 differ, 0 equal, 0 block-dependent (the mismatch is detected and is not the block size) |
| `churn` | `churn -n 200` | 0 | PSS slope 0.04 KB per cycle, 0 threads, 0 descriptors |
| | `--inject leak-kb=16` | 1 | slope 16.1 KB per cycle, only PSS fails |
| | `--inject fd` | 1 | 189 descriptors left, only descriptors fail |
| | `--inject thread` | 1 | 189 threads left, threads fail (and PSS, thread stacks) |
| `bursts` | `--order params-first --capacity 1024 --counts 16,64,256,1024` | 0 | all four trials fully applied, every note ended; burst block 107 us at 1024 events against 2.4 us for a typical block |
| | `--inject drop-after=40` | 1 | "not all applied" from 64 events up; 330 of 346 notes never end at 1024 |
| | `--order notes-first` (a finding, not a control) | 1 | see Findings: lost note-ons from 256 events |
| `note-end` | default script, 5 block sizes | 0 | 15 of 15 notes end once, inside their block, at every block size |
| | `--inject drop-end=3` | 1 | 5 of 15 never ended |
| | `--inject late-end=70` | 1 | 15 `NOTE_END`s past the end of a 64-frame block |
| `process-stats` | `--rt 10 --require-rt`, 20 s | 0 | `SCHED_RR` 10 granted by rtkit and read back |
| | `--rt none`, 20 s | 0 | normal priority runs, with the warning and 444 deadline misses in 15,000 blocks |
| | `--rt 99 --require-rt` | 1 | rtkit refuses (priority 99 over its limit of 20) |
| | `--max-p99-us 1` | 1 | an impossible budget is reported as exceeded |
| | `--inject busy-us=400,every=200 --max-p999-us 333` | 1 | p99.9 412 us over the budget; p99 unchanged at 13.8 us |
| | `--instances 4 --threads 2` | 0 | the multi-instance path |
| | `--editor` under `xvfb-run` | 0 | the same run with an editor open: p99 13.1 us against 13.7 us without |
| `latency` | `--param "Macro 1" --from 1.0 --to 0.2 --max-onset-ms 0.5 --max-audible-ms 2` | 0 | onset 0 frames at every offset (sample accurate), audible within 12 frames |
| | `--param "Slot 1" --from 0.9 --to 0.2` | 0 | a filter parameter: audible within 33 frames |
| | `--param "Macro 2"` | 1 | "no audible change": the parameter does nothing |
| | `--inject delay-blocks=3 --max-onset-ms 1` | 1 | onset 192 frames (4 ms) at every offset |

### Measured on spike-shell

These are mechanism proofs of the host on a small plugin, **not** the G1 numbers of `enth-plugin`; the gate numbers are for the real bundle (lane RT). The machine was not idle for any of them: an i7-9750H laptop (12 logical CPUs), kernel 7.0.0-38 `PREEMPT_DYNAMIC`, `powersave` governor at 2.9 to 3.4 GHz, PipeWire at quantum 2048 and 48 kHz, each timing taken under the exclusive build lock, with other work (not builds) still running: load average **13.5** at the start of the 120 s run, `uptime` "20:54:45 up 17:16". The reports hold the full snapshot.

`process-stats --gate g1`: one spike-shell instance, a 16-note held chord (spike-shell has 8 voices, so it steals), 64 frames at 48 kHz, `SCHED_RR` 10 from rtkit, 120 s after a 10 s warm-up (90,000 blocks), release bundle:

| `process()` | p50 | p90 | p99 | p99.9 | p99.99 | max |
| --- | --- | --- | --- | --- | --- | --- |
| microseconds | 9.3 | 11.9 | **13.9** | **26.7** | 30.5 | 108.4 |
| share of the 1333 us period | 0.70 % | | **1.04 %** | **2.00 %** | | 8.1 % |

Budget 333 us: passed with a 12 to 24 fold margin. The callback around it (the host's event building plus `process()`): p99 18.4 us of thread CPU time, p99.9 31.6 us, max 199 us; wake-up lateness p50 10.5 us, p99 43.9 us, max 2.3 ms; **3 deadline misses in 90,000 blocks**, 0 resynchronisations. The histogram: 33,108 samples in 4 to 8 us, 56,407 in 8 to 16 us, 477 in 16 to 32 us, 5 in 32 to 64 us, 3 in 64 to 128 us.

`process-stats --gate g1-multi`: 16 instances on 4 threads (4 each), 30 s after a 5 s warm-up (22,500 blocks per instance, 360,000 `process()` calls): `process()` p50 6.8 us, p99 11.0 us, p99.9 24.8 us, max 329.5 us. Per thread, the callback p99 is 46.8 to 50.2 us of CPU time, **3.5 to 3.8 % of the period** against the 50 % budget, with 1, 1, 0 and 1 deadline misses. Passed.

The same command at normal priority (`--rt none`, 20 s) gives a `process()` p99 of 14.0 us, the same as at `SCHED_RR`, but **444 of 15,000 blocks (3.0 %) miss their deadline**, with 5 resynchronisations, a wake-up lateness p99 of 2.9 ms and a maximum of 47 ms: a normal-priority thread measures the scheduler, which is why the gate requires real-time priority.

`latency` (block 64, event at 0.5 s at offsets 0, 17 and 63; notes held):

| Parameter | Onset | Audible (-60 dBFS) | Settled (90 %) |
| --- | --- | --- | --- |
| `Macro 1` 1.0 to 0.2 (gain) | 0 frames at every offset | 3 to 12 frames (0.06 to 0.25 ms) | 237 ms |
| `Slot 1` 0.9 to 0.2 (cutoff) | 0 frames at every offset | 2 to 33 frames (0.04 to 0.69 ms) | 367 ms |

spike-shell applies events at their exact frame and smooths over about a quarter second, so the knob-to-sound latency is the smoother's, not the host's block.

Other numbers: `cli-equal` renders 62 cases x 4 block sizes with identical hashes at every block size; `bursts` with params first applies everything up to 1024 events in a 64-frame block (the burst block costs 17 us at 16 events, 44 at 256, 107 at 1024; a typical block 2.4 us); `churn` over 200 cycles of 64 blocks: PSS 4043 KB after warm-up, 4056 KB at the end, 1 thread, 4 descriptors, constant.

## Findings for SK, PL and RT

**For PL (`enth-plugin`).**

1. **nice-plug can lose a note-on that precedes a parameter event on the same frame.** On spike-shell (nice-plug 0.4.2, `SAMPLE_ACCURATE_AUTOMATION = true`), `bursts --order notes-first` loses note-ons from 256 events in a block: 85 of 90 notes never start (and so never get a `NOTE_END`), 337 of 346 at 1024 events; `applied_all_up_to_events` is 64. The same events with the parameter event first on a frame lose nothing up to 1024. Cause as read from the nice-plug source: the block is split at parameter events, each segment drains the input queue with `handle_in_events_until`, and the standard look-ahead `next_event` loop drops what is queued behind an event that ends the segment. A host that sends a note and a parameter change on one frame, note first, can trigger it; whether REAPER does is not known. `enth-plugin` has to avoid it (collect all events of the block before splitting, or keep `SAMPLE_ACCURATE_AUTOMATION = false` and split itself) and prove it with `bursts --order both --capacity <INPUT_EVENT_CAPACITY>` showing the same `applied_all_up_to_events` for both orders.
2. **`NOTE_END` strictness.** `note-end` fails on duplicates, a wrong key, an offset past the end of the block, an end before the note-on and on notes that never end. A stolen voice still has to end its note, and each started note must end exactly once at every block size. Run it with `--state` of a patch that uses the voice count and steals of `enth-plugin`.
3. **State.** `--state some.patch.json` wraps the document into `fields.patch` of an envelope with `params:{}` and no `enth` blob and no parameter values. `enth-plugin` must load a state that has only the `patch` field (and leave every parameter at its default); if the real state also needs the versioned `enth` blob, give the host the full envelope (`--state-format envelope`) or a saved blob. `render --check-roundtrip` shows whether the saved field is byte-identical.
4. **`cli-equal`** compares with the default seed only; the 10 skipped cases (4 seed or stand-in cases, 6 with engine-address automation) are listed with reasons in the report. For the automation cases, write a profile that maps each case's automation to `Slot N=value@seconds` events (the `[automation]` table) once the slots' parameter addresses are known, and raise `--min-compared`.
5. **Latency budget.** If `enth-plugin` keeps the engine's 32-frame control segments, expect `events_land_on_a_grid` with an onset up to 31 frames (0.65 ms), which exceeds the 0.5 ms used for spike-shell above; set `--max-onset-ms` from the design, not from this document.

**For SK (shell and editor).** `process-stats --editor` (Xvfb only) shows what an open editor does to `process()`: on spike-shell nothing visible (p99 13.1 us with the editor against 13.7 without; first frame at 165 ms). It needs a software-GL Xvfb, so it compares runs on this machine; it does not stand in for REAPER's GUI thread behaviour.

**For RT (G1).**

1. The gate is `enth-test-host process-stats <enth-plugin.clap> --gate g1` (and `--gate g1-multi`) under `flock -x /tmp/enth-quiet.lock`; they set rtkit `SCHED_RR` 10, 120 s after 10 s, 64 frames at 48 kHz, p99 and p99.9 below 333 us, the 16-note chord. Add `--state` for the G1 patch. They exit 1 when a budget is exceeded or real-time scheduling was refused.
2. rtkit needs nothing from you here: no sudo, no `ulimit -r`. It lets this user take `SCHED_RR` up to 20 and 25 threads; PipeWire holds four. The command lowers `RLIMIT_RTTIME` to 200 ms for itself; a plugin whose `process()` blocks for 200 ms of CPU time without sleeping is killed rather than freezing the desktop.
3. Record `uptime` and the load average with the numbers (the report does) and say when the machine was not idle. The multi-instance gate needs at least four cores that the machine can spare; 16 instances on one thread are not a valid test on this laptop.
4. **Not covered: the plan-swap row.** "Swap-block p99 with a structural edit every 250 ms" needs a state reload or an edit while the audio thread runs, which the host does not do yet; there is no control for it. `latency` and `bursts` do not replace it.
5. `process-stats` times the wall clock of one `process()` call with the editor closed unless `--editor`. It does not check for allocations or locks in `process()`; that is another tool's job.

## Limits

- **Software GL.** Everything with an editor ran on Mesa llvmpipe under Xvfb. PSS includes Mesa's own growth, the 30 open-state threads include 12 `llvmpipe-N` workers, and a real driver (NVIDIA, AMD, Intel) will behave differently. The keymap leak is independent of the GPU path, but the EGL part and the warm-up size are not. The same run on the real display is on the owner's checklist; this machine must not do it (see the X driver crash on 2026-10-19 in AGENTS.md).
- **Not REAPER.** The host does not reproduce REAPER's behaviours: the restart during an offline render, its parameter flush pattern, where it calls GUI functions from. It has no `thread-check` extension; the `gui` calls run on the thread that owns the instance, which is the right thread but proves nothing about a host that uses another.
- **One GUI path.** Embedded X11 only (all that nice-plug supports). No Wayland, no floating windows, no scale other than 1.0.
- **PSS noise.** PSS moves in steps of a few hundred KB to 2 MB between neighbouring cycles. Read the slope over many cycles after the warm-up and compare it with the baseline; a 20-cycle run cannot see a 15 KB leak.
- **The debug bundle was run once for 200 cycles** (`--repeat 1`, load 5.44 at the start and 3.02 at the end): editor 77.9 KB per cycle against a baseline of 0.19 (difference 77.7 KB), 3 threads closed and 30 open, 200 of 200 drawn, open time median 70.6 ms (p95 85 ms, max 131 ms). The leak does not depend on the build profile; the open time does.
- **Golden hashes are one machine and one compiler.** They hold between the debug and release bundles here. A compiler or dependency bump can change float code generation and legitimately change a hash; run `--update` then and read the diff of the WAV levels. Runtime-detected SIMD would make hashes differ between CPUs. spike-shell has none.
- **nice-plug runs GUI work inline when `process()` is on the main thread.** If a host calls `process()` from the thread that owns the editor, an editor operation can run inside the audio callback. `render --process-on caller` reproduces that arrangement on purpose; it gives the same hash for spike-shell, because no editor is open.
- **The audio thread in `editor-cycles` is real-time paced but not real-time safe;** it is a host loop, not a latency measurement.
- **heaptrack does not work with an open editor** (above). valgrind does, slowly.
- **The REAPER cross-check of the second-instance difference was not done.**
- **P1b: spike-shell is a mechanism proof.** Every number under "Measured on spike-shell" says the host measures and fails correctly; none is a G1 number for `enth-plugin`. The laptop was loaded (load average about 13 on 12 logical CPUs) and runs on the `powersave` governor.
- **P1b: `cli-equal` renders with the default seed only,** and compares an engine patch's hash with the plugin's, which is the same only for the same compiler and float code generation (the engine's expected hashes are one machine and one compiler too). Engine-address automation needs a profile.
- **P1b: `churn` has a floor.** A leak of 1 KB per instance is not told from noise at `--max-kb-per-cycle 1`; 2 KB is. It has no editor (`editor-cycles` has).
- **P1b: `bursts` cannot know a plugin's queue size;** `--capacity` is what the caller says it is. The "applied" check compares a burst with the same events fed one frame per block, which assumes events on different blocks give the same sound as events on one block (true for a sample-accurate plugin, as in spike-shell).
- **P1b: `latency` ends at the event.** It measures from a parameter event arriving at `process()` to the sound; the editor's knob handling, the host's parameter flush and the audio buffer are not in it. The result is in frames; add the buffer for what a musician hears.
- **P1b: `process-stats` is paced offline, not a driver.** The thread wakes on a monotonic timer, so there is no audio driver, no xrun counter and no PipeWire graph. `max` is information (interrupts are charged to the running thread: `IRQ_TIME_ACCOUNTING` is off). The `SCHED_RR` request is capped by rtkit at priority 20 and 25 threads; the command checks the policy it got.
- **P1b: the plan-swap row is not measured.** Swap-block timing with a structural edit every 250 ms needs a state reload during processing; there is no such mode.
- **P1b: the TOML reader is a subset.** `toml_lite` reads `key = value`, tables, arrays of tables, strings, numbers, booleans, arrays and inline tables, and rejects the rest with a line number. It exists because a new dependency changes `Cargo.lock`. The `toml` crate is already in the workspace (`xtask`, golden cases): once the coordinator adds `toml.workspace = true` to `tools/test-host/Cargo.toml`, `toml_lite.rs` can be deleted.

## Reproduce

```
cargo xtask host-render                                  # the golden render (debug)
cargo xtask host-render --release --no-build             # after bundling with --release
cargo run -p enth-test-host -- instances target/bundled/spike-shell.clap \
    --notes "C4@0+1.5:0.8, E4@0+1.5:0.8, G4@0+1.5:0.8" -n 3
env -u DISPLAY -u WAYLAND_DISPLAY xvfb-run -a -s "-screen 0 1920x1080x24" \
    target/release/enth-test-host editor-cycles target/bundled/spike-shell.clap -n 200 --repeat 2 --json-out cycles.json
```

```
# P1b: every mode and its negative control on spike-shell (45 checks; several minutes, mostly the timed runs at the default 20 s)
cargo xtask bundle spike-shell --release && cargo build --release -p enth-test-host
tools/test-host/proof.sh                                    # timed commands take the exclusive lock themselves
flock -x /tmp/enth-quiet.lock env ENTH_PROOF_LOCKED=1 tools/test-host/proof.sh    # or hold it for the whole run
# the gates, as RT will run them on the real bundle:
flock -x /tmp/enth-quiet.lock target/release/enth-test-host process-stats target/bundled/enth-plugin.clap --gate g1 --state g1.patch.json
flock -x /tmp/enth-quiet.lock target/release/enth-test-host process-stats target/bundled/enth-plugin.clap --gate g1-multi --state g1.patch.json
target/release/enth-test-host cli-equal target/bundled/enth-plugin.clap --min-compared 60
target/release/enth-test-host bursts target/bundled/enth-plugin.clap --order both --capacity <INPUT_EVENT_CAPACITY>
```

For timing or memory numbers, run with nothing else building (the numbers above were taken with the other build lane held off by a lock) and record `uptime`. Never run `editor-cycles` outside Xvfb; the command refuses to, and that guard is the reason it exists.
