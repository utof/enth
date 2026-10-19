# Spike: CLAP test host (Phase 0, lane H)

- Date: 2026-10-19
- Code: `tools/test-host` (package `enth-test-host`), `xtask/src/host_render.rs`, CI job `host-render`. Branch `lane-h/test-host`.
- Decision it feeds: how we test the plugin as a plugin, without REAPER. Companion to [shell.md](shell.md) (the REAPER tests).
- Verdict: **the host works and answers the three questions it was built for.**
  1. A plugin-level golden render is cheap (25 ms for 2 s of audio) and bit-stable: debug and release bundles give the same hashes.
  2. The researcher's REAPER observation (a second spike-shell instance renders differently) **does not reproduce** here, in four different driving modes. The plugin has no cross-instance state that changes the sound. The likely cause is on the host side (below), not proven.
  3. The editor open/close loop **leaks about 77 KB per cycle** on spike-shell (Xvfb, Mesa software GL), threads do not leak, and a baseline without the editor leaks 0.2 KB per cycle. About 61 KB of the 77 is one missing `xkb_keymap_unref` in baseview 0.4.1. With that one line patched in a scratch copy the growth is 16 KB per cycle.

Everything with an editor ran under Xvfb with Mesa software GL (llvmpipe), never on the real display. Numbers from that setup are labelled "Xvfb" below and say nothing about a real GPU driver.

## What was built

A minimal CLAP host on `clack-host =0.2.0` (MIT or Apache-2.0, current release 2026-09-12; `clack-extensions =0.2.0` for the extension bindings). About 3,300 lines including tests. It is deliberately not a general host: it loads one `.clap`, drives it the way the tests need and prints JSON.

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
```

Every command prints one JSON document on stdout (pretty only on a terminal). Schemas: `enth.test-host.info/1`, `.render/1`, `.instances/1`, `.editor-cycles/1` (the combined report) and `.editor-run/1` (one process). Errors are JSON on stderr, schema `enth.test-host.error/1`. Exit codes: 0 ok, 1 a check failed (`instances` found a difference), 2 error. Run `enth-test-host --help` for the full option list.

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

`cargo xtask host-render [--release] [--no-build] [--update]` (Linux only) bundles spike-shell, builds the test host, renders the six cases in `tools/test-host/golden/spike-shell.toml` with `DISPLAY` and `WAYLAND_DISPLAY` removed from the environment (48 kHz, block 64, 2 s) and compares each hash with the committed one. It also fails on non-finite samples and on silence, because a golden of silence proves nothing. WAVs go to `target/host-render/`.

| Case | What it covers | SHA-256 of the samples (first 16 hex) |
| --- | --- | --- |
| chord | three held notes, defaults | `1fd6c79cfdded55b` |
| automation | overlapping notes, host automation of Macro 1 and Slot 1 | `9cf94691af319d36` |
| voice-steal | ten notes for eight voices | `54d85267f4e7739e` |
| state-restore | a state blob (macro 1 = 0.9, slot 1 = 0.1) loaded before activation | `d340047cfd33f6a7` |
| cli-params | `--param` before the first block | `9b5805f4a868b5af` |
| reset | `reset()` mid-note, a later note still sounds | `71cad96ded47fd7e` |

Debug and release bundles give the same six hashes, on this machine. A changed hash means the sound, the parameter or state handling, or the host changed. If that is on purpose: `cargo xtask host-render --update` (it rewrites only the `sha256` lines) and say why in the commit message.

The CI job `host-render` (ubuntu-24.04, no display) runs `cargo xtask host-render` and then `enth-test-host instances` with three instances. `actionlint` 1.7.12 is clean.

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

## Reproduce

```
cargo xtask host-render                                  # the golden render (debug)
cargo xtask host-render --release --no-build             # after bundling with --release
cargo run -p enth-test-host -- instances target/bundled/spike-shell.clap \
    --notes "C4@0+1.5:0.8, E4@0+1.5:0.8, G4@0+1.5:0.8" -n 3
env -u DISPLAY -u WAYLAND_DISPLAY xvfb-run -a -s "-screen 0 1920x1080x24" \
    target/release/enth-test-host editor-cycles target/bundled/spike-shell.clap -n 200 --repeat 2 --json-out cycles.json
```

For timing or memory numbers, run with nothing else building (the numbers above were taken with the other build lane held off by a lock) and record `uptime`. Never run `editor-cycles` outside Xvfb; the command refuses to, and that guard is the reason it exists.
