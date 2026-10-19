# Environment snapshot (Phase 0, Task 1)

- Date: 2026-10-19, 09:25 (+03:00), collected by lane Z with read-only commands (`uname`, `lscpu`, `free`, `nvidia-smi --query-gpu`, `prime-select query`, `dpkg -l`, `--version` calls). No builds, nothing installed or changed.
- Every Phase 0 number was measured on this machine. Attach the volatile part (load, free RAM, PRIME mode, PipeWire quantum) to each new result, as [benchmarks.md](../benchmarks.md#measurement-hygiene) asks.

## Hardware and OS

| Item | Value |
| --- | --- |
| Machine | Acer ConceptD 5 laptop |
| CPU | Intel Core i7-9750H, 6 cores / 12 threads (SMT on), AVX2 and FMA; `intel_pstate`, governor `powersave` |
| RAM | 15 GiB; about 11 GiB available at the time of the snapshot (3.6 GiB free, 8.6 GiB cache) |
| Swap | 31 GiB in two 16 GiB swap files, 2.3 GiB in use |
| GPUs | NVIDIA Quadro RTX 3000 Mobile / Max-Q (TU106GLM, 6 GiB) and Intel UHD Graphics 630 |
| NVIDIA driver | 595.99.02, open kernel module (`nvidia-driver-595-open` 595.99.02-0ubuntu0.24.04.1) |
| PRIME mode | `nvidia` (`prime-select query`): the X server and every GL client run on the NVIDIA GPU |
| Display | 1920 × 1080 at 60 Hz, `Xft.dpi` 96 (scale 1.0), measured by research 02 |
| OS | Linux Mint 22.3 (Zena), Ubuntu 24.04 base, glibc 2.39 |
| Kernel | 7.0.0-38-generic, `PREEMPT_DYNAMIC`; no `preempt=` on the command line (the active mode needs root to read: `sudo cat /sys/kernel/debug/sched/preempt`) |
| Desktop | Cinnamon 6.6.9 on X11; X.Org server 21.1.12 |
| `kernel.perf_event_paranoid` | 4, so `perf` is blocked; instruction counts come from valgrind/callgrind instead |
| Load average at snapshot | 2.38, 2.58, 3.40 (Phase 0 timings ran at 2–30) |

The NVIDIA X driver crashed Xorg once on 2026-10-19 while GL windows were opened and closed (`nvidia_drv.so +0x3fc5a`, matching NVIDIA bug 6205955). Agents never open windows on the real display ([AGENTS.md](../../AGENTS.md)).

## Storage

| Item | Value |
| --- | --- |
| Repo and worktrees | `~/code/enth` and `~/code/enth-worktrees/*` on `/home`: ext4 (`rw,relatime`), NVMe, 773 GB, 64 % used |
| Scratch | `~/code/enth-scratch/<lane>/` on the same ext4 volume |
| Before Phase 0 | The repo lived on an NTFS/FUSE USB disk; file-metadata calls dropped from about 5 ms to about 3 µs after the move ([phase-0.md](../phase-0.md#task-1--machine-setup--day-you--agent)) |

## Audio and hosts

| Item | Value |
| --- | --- |
| PipeWire | 1.0.5 (`pipewire-jack` 1.0.5-1ubuntu3.3), WirePlumber 0.4.17 |
| PipeWire clock | `clock.rate` 48000, `clock.quantum` 2048, `clock.min-quantum` 1024, `clock.max-quantum` 8192, no forced quantum or rate (`pw-metadata -n settings`). A 64-sample test needs the quantum forced (see the [owner checklist](owner-checklist.md)) |
| JACK library | `libjack-jackd2-0` and `-dev` 1.9.21 |
| REAPER | 7.82 (2026-10-04) for Linux x86-64, portable install at `~/apps/reaper-7.82/reaper_linux_x86_64/REAPER/reaper` (60-day evaluation). `cargo xtask reaper-test` drives a throwaway copy of it (`ENTH_REAPER` overrides the path). A Wine/Windows REAPER also exists and cannot load Linux plugins |
| Carla | 2.5.8 (`carla` 2.5.8-0ubuntu5): VST3 only, no CLAP host |
| Bitwig, Ardour | Not installed |
| Plugin folders | `~/.clap` and `~/.vst3` do not exist yet; `cargo xtask install <pkg>` creates them |

## Headless GUI stack

| Item | Value |
| --- | --- |
| Xvfb | 21.1.12 (`xvfb` 2:21.1.12-1ubuntu1.8), used through `xvfb-run -a -s "-screen 0 1920x1080x24"` |
| Mesa | 25.2.8 (`libgl1-mesa-dri`, `mesa-vulkan-drivers`): llvmpipe for GL under Xvfb, lavapipe for the `egui_kittest` snapshots |
| libxkbcommon | 1.6.0 (the library behind the baseview keymap leak, [test-host.md](test-host.md)) |

## Rust toolchain

| Item | Value |
| --- | --- |
| rustup | 1.29.1 (2026-08-13) |
| Repo toolchain | 1.99.1 from `rust-toolchain.toml`: rustc 1.99.1 (58cf8f9c3 2026-10-13), cargo 1.99.1 (5f94df478 2026-08-27), profile `minimal` plus rustfmt and clippy |
| Installed targets (1.99.1) | `x86_64-unknown-linux-gnu`, `wasm32-unknown-unknown`, `wasm32-wasip1` |
| Other toolchains | `stable` (default outside the repo), `nightly` = rustc 1.101.0-nightly (8006091eb 2026-10-17, used for the manual RTSan run), `nightly-2026-04-15`, 1.85, 1.88, 1.95.0 |

## Tools

| Tool | Version | Location |
| --- | --- | --- |
| cargo-deny | **0.19.4** on `PATH`; CI and Task 1 pin **0.20.2** | `~/.cargo/bin/cargo-deny`; 0.20.2 binaries in `~/code/enth-scratch/lane-b/tools/bin/` and `~/code/enth-scratch/2026-10-19-phase-0-spikes/05/tools/bin/`. Run `cargo install --locked cargo-deny --version 0.20.2` to match CI |
| cargo-nextest | 0.9.133 | `~/.cargo/bin` |
| cargo-insta | 1.47.2 | `~/.cargo/bin` |
| valgrind | 3.22.0 | `/usr/bin` |
| gungraun-runner | 0.20.0 | `~/code/enth-scratch/lane-b/tools/bin/`, `~/code/enth-scratch/2026-10-19-phase-0-spikes/03/tools/bin/` |
| hyperfine | 1.18.0 on `PATH` (apt, no instruction metrics); 2.0.0 in scratch | `/usr/bin`; `~/code/enth-scratch/lane-b/tools/bin/` |
| heaptrack | 1.5.0 | `/usr/bin` (segfaults once a plugin opens a window, [test-host.md](test-host.md)) |
| wasmtime | 48.0.6, not on `PATH` | `~/code/enth-scratch/lane-b/tools/bin/wasmtime`, `~/code/enth-scratch/2026-10-19-phase-0-spikes/03/tools/bin/wasmtime` (`cargo xtask wasm-hash` reads `$WASMTIME`) |
| clap-validator | 0.4.1, not on `PATH` | `~/code/enth-scratch/2026-10-19-phase-0-spikes/01/tools/bin/clap-validator` |
| Steinberg VST3 validator | SDK 3.8.1, built from source | `~/code/enth-scratch/2026-10-19-phase-0-spikes/01/build-vst3sdk/bin/Release/validator` |
| actionlint | 1.7.12 | `~/.local/bin` |
| sysstat (`pidstat`) | 12.6.1 | `/usr/bin` |
| smem | 1.5 | `/usr/bin` |
| node | 22.22.3 | `~/.local/node-v22.22.3-linux-x64/bin` |
| Python, uv | 3.12.3, uv 0.12.18 | system, `~/.local/bin` |
| git | 2.43.0 | `/usr/bin` |
