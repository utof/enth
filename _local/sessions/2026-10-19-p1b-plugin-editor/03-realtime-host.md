# 03 — Real-time performance in a host (P1b research)

- **Date:** 2026-10-19 (runs 17:58–19:01)
- **Sub-researcher:** 03 (real-time in a host)
- **Brief:** [brief.md](brief.md), questions 1–6.
- **Scratch:** `~/code/enth-scratch/2026-10-19-p1b/03/`. It holds:
  - `notes.md`, the running log;
  - `rtbench/`, an engine timing harness that depends on main `1404239` by path;
  - `ws/`, a scratch workspace: copies of `enth-dsp`, `enth-params`, `enth-patch`, `enth-engine` and `tools/test-host`, with the prototypes below; diffs in `proto-engine.diff` (+117/−8 lines) and `proto-test-host.diff`;
  - `pw/`, a private PipeWire daemon config, a JACK load client and scripts;
  - `rtsan-dl/`, RTSan across `dlopen`;
  - raw outputs in `runs-*.txt` and `runs-ab/*.json`.
- **Evidence markers:**
  - *measured*: run here, command given;
  - *documented*: source code, man page or official docs, linked and dated;
  - *anecdotal*: forum, blog or magazine.

**Environment snapshot for every number below:**

| Item | Value |
| --- | --- |
| CPU and memory | Intel i7-9750H, 6 cores / 12 threads, L2 256 KB per core, L3 12 MB; 15.8 GB RAM, 2.7 GB free, 14.5 GB of swap in use |
| Kernel | 7.0.0-38-generic, `PREEMPT_DYNAMIC` (boot log: "Dynamic Preempt: lazy"), HZ=1000, `NO_HZ_FULL=y`, `IRQ_TIME_ACCOUNTING` **not set** (interrupt time is charged to the interrupted thread) |
| CPU frequency | Governor `powersave`, EPP `balance_performance` |
| GPU | PRIME `nvidia`, Quadro RTX 3000, driver 595.99.02 |
| Audio | PipeWire 1.0.5; the owner's `~/.config/pipewire/pipewire.conf.d/10-fix-stutter.conf` sets quantum 2048, min 1024, max 8192 |
| Limits | `ulimit -r` 0; `ulimit -l` about 2 GB |
| rtkit | active: MaxRealtimePriority 20, MinNiceLevel −15, RTTimeUSecMax 200,000 µs |
| Load average | **7 to 26** throughout: other researchers were building. Every timing ran under `flock -x /tmp/enth-quiet.lock` (builds under `flock -s`, `CARGO_BUILD_JOBS=3`), so wall-clock numbers are upper bounds |

## Answer in brief

1. **SCHED_FIFO is not needed: rtkit already gives this machine's user SCHED_RR (priority ≤ 20) with `ulimit -r 0`.** *(measured, documented)*
   - The audio-thread setup that works:
     1. lower `RLIMIT_RTTIME` to ≤ 200 ms;
     2. call `org.freedesktop.RealtimeKit1.MakeThreadRealtimeWithPID(pid, tid, prio)`;
     3. you get `SCHED_RR|SCHED_RESET_ON_FORK`.
   - That one change matters more than anything the plugin does. For one G1 instance at 64 frames, 60 s:

     | Thread | Deadline misses | p99.9 |
     | --- | --- | --- |
     | SCHED_OTHER (normal scheduling) | 980–2,299 of 45,000 (2–5 %) | 179–663 µs; wake-up lateness up to 95 ms |
     | RR 10 (round-robin real-time via rtkit) | **3 of 45,000** | **166–185 µs**, which is 12–14 % of the 1,333 µs deadline |

   - The engine's own CPU time is the same in both, 152–187 µs at p99.9: the misses come from the scheduler.
2. **The numbers to gate on are p99 and p99.9 of `process()` under RR. Max is not one.** Max (1.1–1.4 ms under RR) is a property of this laptop: interrupts, SMIs and frequency changes, charged to the thread because `IRQ_TIME_ACCOUNTING` is off. Proposed thresholds are under Recommendations.
   - In REAPER, `process()` runs on anticipative-FX worker threads unless the track sets `I_PERFFLAGS` bit 2 *(documented)*. So in-host numbers need that flag plus a plugin-side histogram.
3. **The plan swap is a real problem at 64 frames, and it is cheap to fix.** *(measured)*
   - Today, single-threaded and pinned, a swap block costs 197–265 µs at p50 and up to 549 µs max, against 57–93 µs for a normal block.
   - Paced under RR, with a structural edit every 250 ms: swap block p99 **797–1,052 µs**, max **997–1,057 µs**, which is **75–79 % of the deadline** for a single instance.
   - It is a memory-bandwidth cost (0 page faults): a 1.6 MB memcpy alone is 148–249 µs.
   - **Prototype 1, implemented and measured:** on an instant swap, move the reverb's delay lines (`mem::swap` of boxed slices) instead of copying them. The swap block drops to p50 74–109 µs and max 120–282 µs (paced: p99 139 µs, max 149 µs).
   - **Prototype 2:** crossfade *before* the effects when the effect list is unchanged, so the effect state is moved too. Crossfade swap block: max 1,057 → 195 µs paced. Crossfade blocks: about 1.3–1.6× a normal block instead of 1.6–2×.
   - The engine's swap, noalloc, segment and voices tests all pass on the prototype. That includes "an unrelated edit is bit-identical" with a patch that has a reverb and a compressor.
4. **Multiple instances.** One G1 instance costs about **100 µs per 64-frame block when paced** (about 7.5 % of a core). That is 1.5–1.8× the offline figure: cold caches after sleeping and frequency scaling. *(measured)*

   | Instances × threads (RR) | Callback p50 | Max | Result |
   | --- | --- | --- | --- |
   | 8 × 1 | 788 µs (59 %) | 1,214 µs (91 %) | 0 misses in 30 s |
   | 16 × 1 | — | — | Cannot keep up: rtkit's RTTIME limit killed the thread |
   | 16 × 4 | 423–426 µs | 600–931 µs | 0–1 misses per thread |

   With 8 RT threads, some threads still saw wake-ups 20–35 ms late.
5. **Editor cost on the real NVIDIA display:** an owner-attended plan is under Recommendations. It runs REAPER against a **private, hardware-less PipeWire daemon at 64 frames**, so the owner's audio configuration and speakers are untouched.
   - Under Xvfb, measured here: the audio-thread interference A/B, with llvmpipe as a worst-case CPU competitor.
     - With an RR audio thread, opening an animating 1100×768 editor made no measurable difference: 0 and 2 misses closed, 4 and 3 open, per 45,000 blocks.
     - With SCHED_OTHER, misses roughly doubled: 1,024–1,146 closed against 1,947–2,114 open, per 22,500 blocks.
6. **xruns** *(measured unless marked)*:
   - **A private PipeWire daemon** (no ALSA nodes, timer-driven Dummy-Driver, quantum 64, own socket) plus `pw-jack -r` gives agents a 64-frame JACK graph with no sound and no change to the owner's PipeWire. Two traps:
     - PipeWire 1.0.5's `module-rt` silently fails through xdg-desktop-portal on this machine. It needs `rtportal.enabled = false`.
     - Dummy-Driver overruns do not raise JACK xrun callbacks. Count them with `pw-top -b` (ERR).
   - **Allocation regressions:** keep `assert_process_allocs` on debug bundles. It wraps nice-plug's whole process wrapper, debug only *(documented)*.
   - **Add a nightly RTSan test host:** an RTSan-built executable linked with `--export-dynamic` that marks the plugin call real-time catches `malloc` inside an *uninstrumented, stable-built* cdylib (measured, nightly 1.101.0).
7. **Before P1b ships:**
   - the move-on-instant-swap fix;
   - the test host's `process-stats` command (prototype exists) with rtkit RR;
   - the plugin-side real-time histogram;
   - the swap-block condition in the G1 check.

   Pre-effects crossfade: P1b if the owner accepts the changed crossfade definition, otherwise S1. Defer or accept the rest (numbers under Recommendations).

## Findings

### F1. Getting a real-time thread without `rtprio` (Q1)

*Measured:*
- `busctl get-property org.freedesktop.RealtimeKit1 ...` gives MaxRealtimePriority 20, MinNiceLevel −15, RTTimeUSecMax 200000.
- `ps -eLo cls,rtprio,comm` shows PipeWire's four `pw-data-loop` threads already at RR 20, and another application's audio threads at RR 10.
- `rtbench blocks --rt 10` sets `RLIMIT_RTTIME` to 200 ms, then runs `busctl call --system org.freedesktop.RealtimeKit1 /org/freedesktop/RealtimeKit1 org.freedesktop.RealtimeKit1 MakeThreadRealtimeWithPID ttu <pid> <tid> 10`. `sched_getscheduler` then returns `SCHED_RR|SCHED_RESET_ON_FORK`.

*Documented* (rtkit source, [`rtkit-daemon.c`](https://raw.githubusercontent.com/heftig/rtkit/master/rtkit-daemon.c), read 2026-10-19):
- defaults: `max_realtime_priority = 20`, `rttime_usec_max = 200000`, `min_nice_level = -15`;
- **`threads_per_user_max = 25`**;
- the policy is `SCHED_RR|SCHED_RESET_ON_FORK` unless the daemon is started with `--scheduling-policy=fifo`;
- the target must have a hard `RLIMIT_RTTIME` ≤ 200 ms, else the call fails with `EPERM`;
- a canary watchdog (cheep every 5 s, watchdog 10 s) demotes the threads rtkit knows of and refuses new requests for 5 minutes if it starves.

Consequences:
- **With `RLIMIT_RTTIME` at 200 ms, an RR thread that never blocks is killed.**
  - Measured: 16 G1 instances on one thread could not finish a 64-frame callback inside its period, so the loop never slept and the process died with "Killed" (soft limit = hard limit, so SIGKILL).
  - That is a feature: a runaway test cannot freeze the desktop.
- The journal showed rtkit "Supervising 22–23 threads" during the 8-thread run. **The 25-per-user cap is close** once PipeWire (4) and other applications hold RT threads, so test runs should use few RT threads.
- SCHED_FIFO would need the owner: `limits.conf` rtprio or rtkit's daemon flag, which is a system setting. It is not needed: a thread that sleeps every period behaves the same under RR and FIFO (RR's 100 ms slice only matters between equal-priority threads that do not block).

**One G1 instance, paced 64-frame loop, 60 s each** (`rtbench blocks --secs 60 [--pin 3] [--rt 10]`, `runs-blocks-1inst.txt`, 18:10–18:14, load 8–15). Lateness is how late the thread woke after `clock_nanosleep(TIMER_ABSTIME)`. A miss is lateness + callback time > 1,333 µs. All times in µs.

| Thread | wall p50 | p99 | p99.9 | p99.99 | max | CPU p99.9 | lateness p99 / p99.9 / max | misses |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| SCHED_OTHER | 87.5 | 144.1 | 662.9 | 3,688 | 5,575 | 161.4 | 2,380 / 7,265 / 95,034 | 980 / 45,000 |
| SCHED_OTHER, pinned to CPU 3 | 96.2 | 134.3 | 178.5 | 2,648 | 20,984 | 152.8 | 3,108 / 9,170 / 34,745 | 2,299 |
| **RR 10 (rtkit)** | 105.9 | 145.9 | **185.4** | 648.7 | 1,075.5 | 187.1 | 43.0 / 55.6 / 1,466 | **3** |
| RR 10, pinned | 104.9 | 138.6 | **166.2** | 331.6 | 1,369.9 | 164.4 | 41.6 / 56.7 / 1,934 | **3** |

- Pinning without isolation makes SCHED_OTHER *worse* (2,299 misses): a pinned thread cannot move off a busy CPU. Use RR without pinning, or RR with pinning for slightly tighter tails.
- Under RR the worst blocks have CPU time ≈ wall time (max(wall − cpu) 10 µs, unpinned): they are interrupts, SMIs or frequency dips charged to the thread, not preemption.

**Block size, RR 10, 15 s each** (`runs-blocks-sizes.txt`, load 19–26; µs):

| Block | p50 | p99.9 | max | Note |
| --- | --- | --- | --- | --- |
| 32 | 56.4 | 77.4 | 14,860 | 24 misses / 22,500 from 14 ms wake-ups |
| 48 | 64.3 | 129.3 | 155.5 | Bimodal: p90 113. A 48-frame block renders one or two 32-frame segments, so the worst block always costs ⌈B/32⌉ segments |
| 64 | 102.4 | 134.8 | 139.3 | |
| 128 | 202.9 | 260.5 | 294.1 | |
| 256 | 393.1 | 468.4 | 489.9 | |
| 512 | 809.2 | 971.8 | 972.4 | |

Free-running at the same load: 64 frames p50 67.3 µs, 512 frames 718.7 µs.
- At 64 frames the paced block costs **1.5× the free-running one** (102 against 67 µs).
- Per frame the free-running cost is 1.05–1.40 µs, against 0.885 µs in the G1 table, which was taken at load about 7. Load inflates everything here.

### F2. In REAPER: what `process()` timing means (Q1)

- **Anticipative FX.**
  - *Documented:* REAPER's API documents `I_PERFFLAGS : int * : track performance flags, &1=no media buffering, &2=no anticipative FX` ([ReaScript API help](https://www.reaper.fm/sdk/reascript/reascripthelp.html), generated for REAPER 7.42, fetched 2026-10-19). The REAPER 7.82 binary has the same string, plus "Anticipative FX processing - superior multiprocessing and lower interface latencies" and "Render-ahead:" in its preferences.
  - *Anecdotal:*
    - Sound On Sound ([Running multiple plug-ins](https://www.soundonsound.com/techniques/running-multiple-plug-ins), July 2011) says anticipative processing runs FX "at irregular intervals, often out of order, and slightly ahead of time", and to turn it off for low-latency input monitoring.
    - A search snippet of the Cockos wiki ([Preferences Audio Buffering](https://www.cockos.com/wiki/index.php/Preferences_Audio_Buffering); the page itself is behind a bot check, not fetched) gives a 200 ms default render-ahead.
    - A [KVR post](https://www.kvraudio.com/forum/viewtopic.php?p=4950007) from the REAPER 4 era believes it is disabled automatically on monitored, record-armed tracks.
  - Consequence: with a MIDI item playing, the plugin's `process()` is *not* on the device deadline. Its duration is still CPU, but xruns need `I_PERFFLAGS |= 2` (Lua: `reaper.SetMediaTrackInfo_Value(tr, "I_PERFFLAGS", 2)`) or live input. REAPER's `IsInRealTimeAudio()` API says which thread a call is on.
- **How to measure in REAPER:** REAPER does not expose per-call plugin timing (only its Performance Meter, which is GUI). So the plugin should measure itself:
  - an `rt-stats` cargo feature, off by default;
  - per `process()` call, `Instant` wall time into a fixed log-bucketed atomic histogram;
  - a JSON dump at `deactivate()` on the main thread, to a path read once at `initialize()` (never `env::var` inside `process()`: shell.md pitfall 4).

  The same histogram then works in REAPER, Carla, Bitwig and the test host. It was not built here: there is no `enth-plugin` yet.
- **Dummy Audio:** whether REAPER's Dummy Audio thread is real-time paced and at what priority was not verified (Gaps).

### F3. Plan-swap cost (Q2)

The engine code (main `1404239`):
- `Plan::carry_from` copies every surviving module's state on the audio thread.
- `MultiBuffer::copy_state_from` is `self.buf.copy_from_slice(&other.buf)` over the reverb's six delay rings, 1,638,400 bytes at 44.1 and 48 kHz.
- During a crossfade (a module added or removed), both plans run steps 3–8, the reverb included, for 256 frames.

**Raw memory cost** (`rtbench memcpy --reps 200 [--pin 3]`, `runs-memcpy.txt`, load 14):

| Operation | Pinned p50 / p99 | Unpinned p50 |
| --- | --- | --- |
| 1.6 MB copy, cold cache | 229 / 359 µs | 249 µs |
| Same, hot | 148 / 226 µs | 165 µs |
| `fill(0.0)` | 58 µs | 67 µs |

**Swap blocks, single thread, pinned** (`rtbench swap --swap instant|xfade --reps 200–300 --pin 3`: 40 normal 64-frame blocks, then a plan sent from the same thread, then the swap block and the crossfade blocks; `runs-swap.txt`, `runs-swap-move.txt`, 18:03–18:10, load 10–14). The page-fault count in the swap block (`getrusage(RUSAGE_THREAD)`) is **0**: pure bandwidth, not first-touch faults. `mlockall` did not change it.

| Case | Normal block p50 | Swap block p50 / p99 / max | Crossfade blocks p50 / max |
| --- | --- | --- | --- |
| main, instant swap (recompile, nothing added) | 57–93 | **197–265 / 314–471 / 319–549** | — |
| main, crossfade swap (add or remove an unrouted LFO) | 61–87 | **256–348 / 378–511 / 416–586** | 124–174 / 194–276 |
| main, G1 without reverb, instant | 47 | 58 / 69 / 74 | — |
| main, G1 without reverb, crossfade | 41 | 92 / 124 / 127 | 82 / 118–132 |
| **Prototype, instant (move)** | 60–89 | **74–109 / 116–149 / 120–282** | — |
| **Prototype, crossfade (pre-effects, moved)** | 64–86 | **114–148 / 175–187 / 179–191** | **97–128 / 153–167** |

One 2.15 ms outlier in a prototype crossfade block, and 2.3 ms ones in normal blocks of both builds, are SCHED_OTHER preemption on a pinned, loaded CPU.

**Paced, RR 10, a plan sent every 250 ms from another thread, 30 s** (`rtbench blocks --rt 10 --swap instant|xfade --swap-ms 250`, `runs-blocks-swap-paced.txt`, 18:52, load 13–17):

| Build | Swap block p50 / p99 / max | Crossfade blocks p50 / max | Blocks > 25 % of the deadline |
| --- | --- | --- | --- |
| main, instant | 328 / 797 / **997** | — | 53 |
| prototype, instant | 103 / 139 / **149** | — | 3 |
| main, crossfade | 354 / 1,052 / **1,057** | 174 / 294 | 77 |
| prototype, crossfade | 160 / 189 / **195** | 144 / 176 | 3 |

- Paced, the copy is 2–4× slower than in the pinned loop. The new plan's 1.6 MB was just zeroed by the main thread on another core, the old plan's lines are cold after the sleep, and the core clocks down between bursts.
- **A single structural edit can take 0.8–1.06 ms of a 1.33 ms period.** Next to other plugins on the same thread, that is an xrun.

**The prototypes** (`proto-engine.diff`, +117/−8 lines in 7 files, for the measurement only):

1. **Move on an instant swap.** `carry_from` first runs `pair` over all five role lists (cheap) to learn whether the swap will crossfade.
   - If not, the old plan is never run again (it goes straight back on the return ring). So each surviving effect takes its state with `GlobalProcessor::take_state_from(&mut old)`, whose default is `copy_state_from`.
   - The reverb overrides it with an O(1) exchange; `carry_from` takes `&mut Plan`. The heart of it (enth-dsp `reverb/delay.rs`):

     ```rust
     fn swap_state_with(&mut self, other: &mut Self) -> bool {
         if self.buf.len() != other.buf.len() || self.stride != other.stride { return false; }
         core::mem::swap(&mut self.buf, &mut other.buf);
         self.index = other.index;
         true
     }
     ```

   - Nothing is freed on the audio thread: the old plan leaves with the new plan's unused zeroed rings and is dropped on the main thread by `Link::collect`, as today.
2. **Pre-effects crossfade.** When the effect list pairs one to one (same ids, kinds and order), the crossfade mixes the two plans' *voice sums* after step 7, and one effect chain (the new plan's, with moved state) runs step 8 once.
   - Otherwise (an effect added or removed), it falls back to today's post-effects crossfade with copies. `render_plan` gains a `run_fx` flag; step 8 moved into `render_fx`.
   - Semantics: an unrelated edit stays bit-identical, because when both sums are equal, `old + (new − old)·w` is `old` exactly.
   - What changes: the compressor and the reverb see the blended dry sum instead of two effect chains being blended. Arguably better (one compressor envelope, no two diverging gain states), but it is a definition change for engine.md and design.md.

Verification *(measured)*:
- `cargo test -p enth-engine --test swap --test noalloc --test segment --test voices` in `ws/` passes, 37 tests. That includes:
  - `a_held_note_survives_recompiling_the_same_patch_bit_for_bit` and `a_held_note_survives_an_unrelated_structural_edit`, both with `every_kind_patch` (compressor and reverb), now through the move and pre-effects paths;
  - `a_swap_while_the_return_ring_is_full_keeps_the_plan`;
  - the `assert_no_alloc` overflow tests.
- The prototype's on/off switch is a `static AtomicBool`, read on the audio thread; an earlier draft read an environment variable there, which allocates.

Options compared:

| Option | Cost at the swap | Crossfade cost | Bits | Effort | Verdict |
| --- | --- | --- | --- | --- | --- |
| Today: copy | +140–210 µs pinned; up to +950 µs paced | 1.6–2.0× | — | — | Too slow at 64 frames |
| **Move on an instant swap** | +15–20 µs | unchanged | identical | ~60 lines | **Do it in P1b** |
| **Pre-effects crossfade with moved effect state** | swap block 114–195 µs | 1.3–1.6× | unrelated edits identical; crossfade shape changes | ~60 lines + docs + one test pinning the new form | **P1b if the owner accepts the definition, else S1** |
| Copy incrementally over N blocks before the swap | spread, same bandwidth | unchanged | needs dirty-region tracking of six rings written every sample | high | No |
| Effect state outside plans (engine-owned slots by module id) | O(1) for every surviving effect | needs two slots during a post-effects crossfade anyway | identical | refactor of plan and compile | Only if S2 adds many large-state effects (delay, chorus, CloudSeed) |

Remaining audio-thread passes over 1.6 MB, from the code:
- `reset()` of an *added* reverb clears a freshly compiled, already-zero network (`fill`, about 58–67 µs).
- Engaging the monitor bypass clears it too.
- The idle scan `is_state_zero` costs "about 40 µs" by the code's own comment, first after 0.68 s of silence, then backing off up to 22 s.

All are rare, user-triggered or below 5 % of the deadline: accept, or skip the clear for a module that never ran.

**Compile time** (main thread, `rtbench compile --reps 100`, `runs-compile.txt`, load 18): G1 p50 914 µs, p90 970 µs (preparing the reverb dominates); G1 without reverb p50 55 µs; one first-call outlier of 65–92 ms (warm-up). Compiling off the audio thread is fine for UI latency.

### F4. Multiple instances (Q3)

`rtbench blocks --rt 10 -n N --threads T --secs 30` (`runs-blocks-multi.txt`, 18:19–18:21, load 13–25). Engines are spread round-robin; each thread has its own independent 1,333 µs timer, unlike REAPER's synchronised worker pool. All times in µs.

| Instances × threads | Callback p50 | p99 | p99.9 | max | Misses (per thread) |
| --- | --- | --- | --- | --- | --- |
| 8 × 1 | 788 | 925 | 1,137 | 1,214 | 0 |
| 16 × 1 | — | — | — | — | **Killed by RLIMIT_RTTIME**: never slept, more than 100 % of a core |
| 8 × 4 (2 per thread) | 195–197 | 236–270 | 266–303 | 700–2,472 | 1–5 |
| 16 × 4 (4 per thread) | 423–426 | 472–477 | 519–527 | 600–931 | 0–1 |
| 16 × 8 (2 per thread) | 212–220 | 244–249 | 262–279 | 299–3,084 | 0, 0, 0, 1, 23, 30, 11, 18 |

- Per instance-block, paced: 98–108 µs, about **7.5 % of a core per G1 instance at 64 frames** (the G1 table's offline 4.3 % was at load 7).
- With 8 RT threads plus PipeWire's and another application's RT threads, four of the eight saw wake-up lateness of 10–35 ms (p99.99 8–32 ms). Unexplained: candidates are equal-priority RR contention with the other application's RR 10 threads and the rtkit cap (Gaps).
- **Scheduling noise on this laptop**, the short version:

  | Thread | Wake-up lateness |
  | --- | --- |
  | SCHED_OTHER | p99 2–3 ms, max up to 95 ms |
  | RR | p99.9 about 50 µs, with a 1.5–2.4 ms spike about once a minute |

  The owner's "fix-stutter" config (min quantum 1024) suggests the machine has stuttered at small quanta before *(anecdotal, from the file name)*.

### F5. Editor cost: what Xvfb can show (Q4) and the audio A/B (Q5)

**Test host `process-stats` prototype** (`proto-test-host.diff`, about 300 lines, in `ws/tools/test-host`).
- A new subcommand runs a paced audio thread with optional rtkit RR and pinning.
- It records per block:
  - wall time of the whole callback;
  - wall time of the plugin's `process()` alone (an `Instant` around the CLAP call in `JobRunner::process_block`);
  - thread CPU time;
  - wake-up lateness and misses.
- `--editor` opens the plugin editor in a host-made X11 parent for the whole run, only under Xvfb (the existing `require_xvfb` guard), servicing main-thread callbacks every 1 ms.
- Output is one JSON document, `enth.test-host.process-stats/0-prototype`.
- Run by `ab-editor.sh`, which alternates closed and open, under `xvfb-run` with `__GLX_VENDOR_LIBRARY_NAME=mesa LIBGL_ALWAYS_SOFTWARE=1 __EGL_VENDOR_LIBRARY_FILENAMES=.../50_mesa.json`.

**The A/B:** spike-ui-plugin (the lane U1 release bundle: the 1100×768 reference panel animating its scopes with an 8-note chord), Xvfb with llvmpipe, which is a harsher CPU competitor than a GPU (12 rasteriser threads). All times in µs.

| Audio thread | Editor | Plugin `process()` p50 / p99.9 / max | Lateness max | Misses |
| --- | --- | --- | --- | --- |
| RR 10, 60 s × 2 (`runs-ab-rr10.txt`, load 8–11) | closed | 6.7–9.7 / 21.9–49.6 / 201 | 340 / 1,921 | **0, 2** of 45,000 |
| | open | 9.6–9.9 / 26.3–27.5 / 74–247 | 1,866 / 2,196 | **4, 3** |
| SCHED_OTHER, 30 s × 2 (`runs-ab-other.txt`, load 9–17) | closed | 11.5–11.6 / 26.8–28.0 / 168–341 | 14–33 ms | **1,024, 1,146** of 22,500 |
| | open | 9.8–10.6 / 23.0–28.9 / 341–1,027 | 15–95 ms | **1,947, 2,114** |

- With an RR audio thread, an animating software-rendered editor adds no measurable misses or `process()` time: the run-to-run spread is as large as the difference.
- With SCHED_OTHER it roughly doubles the misses.
- So **GUI-caused xruns on Linux come from scheduling class, locks shared with the audio thread, or GPU-driver latency**, not from editor CPU as such. Only the last needs the real GPU.
- Detecting a GUI effect needs 10-minute interleaved runs: at 2–4 misses a minute, a 60 s A/B cannot resolve a difference.

What Xvfb can and cannot measure (Phase 0 numbers in benchmarks.md, plus the above):

| Under Xvfb (agents) | Owner tier only |
| --- | --- |
| GUI-thread CPU per frame before the GPU (`CLOCK_THREAD_CPUTIME_ID`) | GPU time, SM %, VRAM |
| Achieved fps, wake-ups, PSS, leaks, threads, open time | NVIDIA interrupt and driver stalls on the audio thread |
| **Audio-thread A/B against an editor** (`process-stats --editor`; llvmpipe is the worst case for CPU contention) | Compositor (Cinnamon) cost, vsync and present pacing |
| `execute_gui` inline in `process()` (`render --process-on caller` with a debug bundle) | First-open behaviour, the Xorg crash exposure |

### F6. xruns and allocation regressions (Q5)

**A private PipeWire daemon for agents** (`pw/enth-rt-test.conf`, `pw/start-daemon.sh`, `pw/run-jack.sh`; measured 18:35–18:40):
- The config: `core.name = enth-rt-test`, quantum 64, no ALSA devices, only `support.node.driver` (Dummy-Driver, timer-driven) and `module-rt` with `rt.prio = 20, rtportal.enabled = false`.
- JACK clients run through `PIPEWIRE_REMOTE=enth-rt-test PIPEWIRE_CONFIG_DIR=pw/conf pw-jack -r enth-rt-test <client>`. `pw/conf/jack.conf` is a copy with `rt.prio = 19, rtportal.enabled = false`.
- Nothing can reach hardware, and the owner's daemon, quantum and config files are untouched.
- `pw/jload` (a C JACK client that burns N µs per cycle and registers an xrun callback) reports `buffer=64`, 750 cycles/s.
- **Trap:** with defaults, PipeWire 1.0.5's `module-rt` logged "acquired realtime priority 20 ... using RTKit", but the journal shows xdg-desktop-portal's "Realtime error: Could not get pidns: pidns required but no pidfd provided", and `/proc/<tid>/sched` stayed SCHED_OTHER. With `rtportal.enabled = false` (direct rtkit), the daemon's data loop is RR 20 and the client's is RR 19.

| jload burn per 1,333 µs cycle | RT | Result |
| --- | --- | --- |
| 600 µs | no | ERR 3 in about 2 s (`pw-top -b -r enth-rt-test`); max cycle 920 µs |
| 600 µs | yes | ERR 0, 4,500 of 4,500 cycles, max cycle 603 µs |
| 1,400 µs (above the period) | yes | **0 JACK xrun callbacks**; the graph slowed instead (2,130 cycles in 3 s rather than 2,250) |

So under Dummy-Driver count misses with `pw-top` ERR (*documented* in `man pw-top`, PipeWire 1.0.5: "Xruns for followers are incremented when the node started processing but did not complete before the end of the graph cycle deadline"), and cycles completed against expected. Not with the JACK callback.

**REAPER as a JACK client of the private daemon:** not achieved headless.
- `linux_audio_mode=1` and `=0` in a throwaway `reaper.ini`: REAPER mapped `libasound`, not `libjack`, and no node appeared.
- The JACK value is unknown. Probing further was stopped because a PulseAudio mode would connect to the owner's real audio server.
- The owner can pick JACK in the preferences dialog instead (Recommendations, owner tier).

**The owner's real device:**
- *Documented* (`man pipewire`, 1.0.5): settings metadata "can bypass any of the restrictions listed in the config options above, such as quantum". So `pw-metadata -n settings 0 clock.force-quantum 64` works despite the owner's min-quantum 1024; reset it to 0 afterwards.
- `pw-jack -p 64` only sets `PIPEWIRE_QUANTUM` (read from `/usr/bin/pw-jack`), a request that the config's limits clamp.

**Allocation checks:**
- *Documented:* nice-plug 0.4.2 `src/wrapper/util.rs` `process_wrapper` sets FTZ and, only under `cfg(all(debug_assertions, feature = "assert_process_allocs"))`, wraps the **whole** CLAP and VST3 process callback, wrapper included, in `nice_assert_no_alloc::assert_no_alloc` (cargo registry source; [Codeberg](https://codeberg.org/RustAudio/nice-plug)). So debug bundles under the test host's `render`, `process-stats` and clap-validator catch allocations; release bundles never do.
- **RTSan across the plugin boundary** *(measured, nightly 1.101.0 2026-10-17, `rtsan-dl/`)*:
  - *Documented* ([rustc `link.rs`](https://github.com/rust-lang/rust/blob/master/compiler/rustc_codegen_ssa/src/back/link.rs), read 2026-10-19): on Linux the sanitizer runtimes "are currently distributed as static libraries which should be linked to executables only", so a cdylib never carries the RTSan runtime.
  - A cdylib built with `-Zsanitizer=realtime` has undefined `__rtsan_realtime_enter/exit` and fails to `dlopen` from an RTSan executable unless that executable is linked with `-Clink-arg=-Wl,--export-dynamic`. With it, an allocation in a `#[sanitize(realtime = "nonblocking")]` function is reported (exit 43).
  - **Better: the host marks the call itself.** An RTSan-built executable (with `--export-dynamic`) that calls `__rtsan_realtime_enter()` before and `__rtsan_realtime_exit()` after the plugin call reports "Intercepted call to real-time unsafe function `malloc`" from a **plain stable 1.99.1 cdylib with no attributes**.
  - In the same test, an uncontended `std::sync::Mutex` and a `thread::sleep` were *not* reported.
  - `__rtsan_realtime_enter` is not in the public API: [Clang's RTSan docs](https://clang.llvm.org/docs/RealtimeSanitizer.html) document only `[[clang::nonblocking]]` and `__rtsan_disable/enable`. Treat it as an internal symbol that can change.

## Recommendations

### Thresholds (G1 now, GS1 later)

- **G1, agent tier** (replaces "p99 `process()` < 25 % in the test host"). Conditions:
  - the real `enth-plugin` release bundle;
  - the G1 patch, a held 16-note chord;
  - 64 frames at 48 kHz, `process-stats` with rtkit RR 10, 120 s, one instance, after a 10 s warm-up;
  - environment snapshot recorded.

  Pass:
  - p99 < 25 % (333 µs) **and** p99.9 < 25 % of the deadline;
  - **with structural edits every 250 ms** (instant and crossfading swaps, alternating), swap-block p99 < 25 % and crossfade-block p99 < 25 %.

  Report max, p99.99, lateness, misses and max(wall − cpu) as information.

  Today (the engine standing in for the plugin): steady p99 125–146 µs and p99.9 166–186 µs pass. Swap blocks fail on main (p99 797–1,052 µs) and pass with the move and pre-effects prototypes (139–189 µs).
- **Deterministic guards in CI**, since wall clock is information only:
  - a test that an instant swap moves rather than copies: the new plan's reverb ring pointers equal the old plan's;
  - gungraun instruction budgets for three spiky blocks: one with an instant swap, one with a crossfade, one with 16 note-ons.

  Instruction counts under-state memcpy (memory-bound), so the move test, not the budget, guards the copy.
- **Multi-instance (information):** 16 instances on 4 RR threads, each thread's p99 < 50 %. Measured 472–477 µs (35–36 %).
- **GS1:**
  - the one-hour REAPER soak with the **debug** bundle (`assert_process_allocs`) and every plugin track at `I_PERFFLAGS = 2`;
  - the nightly RTSan test-host run with 0 reports;
  - a 10-minute × 2 interleaved `process-stats` A/B, editor closed and open (Xvfb, RR): misses(open) ≤ misses(closed) + 2·√(misses(closed)) + 2, and p99.9(open) ≤ 1.1 × p99.9(closed) + 20 µs;
  - the owner-tier version of the same on the real GPU.

### Fix before P1b ships

1. **Move the effect state on instant swaps** (prototype 1).
   - Measured: swap block p50 197–265 → 74–109 µs pinned; paced max 997 → 149 µs.
   - Keep `copy_state_from` for the crossfade path. Add `take_state_from` with a copying default, so a module lane that does not override it stays correct.
2. **Test host `process-stats`** (the prototype, cleaned up):
   - rtkit RR via D-Bus. Use `zbus` or shell out to `busctl`: a tool, exempt from layers, but check the licence;
   - `--editor` (Xvfb only), `--pin`, `--json-out`;
   - the plugin-only `process()` time;
   - an environment snapshot in the JSON (load, governor, quantum, kernel, preempt mode).

   And an `--instances N --threads T` mode, as `rtbench` has.
3. **The plugin-side real-time histogram** behind a cargo feature, for REAPER, Carla and Bitwig. REAPER harness scripts set `I_PERFFLAGS` bit 2 and check `IsInRealTimeAudio()`.
4. **Wake-up lateness and RR in every timing run.** SCHED_OTHER runs measure the scheduler (2–5 % misses), not the plugin. Say so in benchmarks.md.

### Should do (P1b if time allows; otherwise early S1)

5. **Pre-effects crossfade** (prototype 2).
   - Measured: crossfade swap block max 1,057 → 195 µs paced; crossfade blocks 174 → 144 µs p50.
   - Needs:
     - the owner's or coordinator's OK on the changed crossfade definition;
     - an engine.md and design.md update;
     - a test pinning "blend the voice sums, then one effect chain".
   - Keep the copy fallback for effect-list changes.
6. **Nightly RTSan over the plugin** in the test host:
   - a `rtsan` feature that wraps the CLAP `process()` call in `__rtsan_realtime_enter/exit`;
   - build the test host with `RUSTFLAGS="-Zsanitizer=realtime -Clink-arg=-Wl,--export-dynamic"` on nightly;
   - run `render` and `process-stats --secs 20` against the stable-built debug and release bundles.

   It covers nice-plug's wrapper, the shell's translation code and the engine together, which `--cfg enth_rtsan` on the engine alone does not. Pin the nightly date: the entry symbol is internal.
7. **Private PipeWire daemon harness** (`pw/`), to turn G0's "no GUI-caused xruns" (pending owner) into a partly agent-run check:
   - `pw-top -b -r enth-rt-test` ERR plus cycle counts, editor open and closed, 10 minutes each, interleaved;
   - REAPER's JACK setting still to be found (Gaps);
   - meanwhile `process-stats --editor` is the agent-tier proxy.
8. Skip `reset()`'s 1.6 MB clear for an effect module that has never run (saves 58–67 µs when a reverb is added).

### Defer or accept (with numbers)

- The copy when the effect list itself changes while a reverb survives: a swap block like today's (256–586 µs pinned, up to about 1 ms paced). Rare (adding or removing a compressor); revisit with state outside plans if S2 adds large-state effects.
- Reverb idle scan (about 40 µs, rare) and the bypass-engage clear (58–67 µs): under 5 % of the deadline.
- The 1–2.4 ms system spikes under RR: machine property (interrupts charged to the thread, `powersave` governor). Do not gate on max.
- 16 G1 instances on one thread at 64 frames: not possible on this laptop (more than 100 % of a core paced). Document about 7.5 % of a core per G1 instance at 64 frames paced (load 10–25), 4.3 % offline.
- SCHED_FIFO, `isolcpus`/`nohz_full`, the performance governor: owner-only system changes. Not needed for A/B; optional for an absolute quiet-machine baseline.

### Owner-attended plan: editor CPU and GPU on the real display (Q4) and real xruns (Q5)

Save your work first: the NVIDIA 595 X driver can take Xorg down. Wait at least 10 s between opening windows. Never script open/close cycles on the real display.

```bash
# 0. Snapshot (no windows)
uptime; nvidia-smi --query-gpu=name,driver_version,pstate,clocks.gr,power.draw --format=csv; prime-select query
pidstat -p "$(pgrep -x cinnamon)" 1 10          # compositor baseline; restart Cinnamon (Ctrl+Alt+Esc) if near 100 %
# 1. A hardware-less 64-frame graph (your PipeWire and speakers untouched)
~/code/enth-scratch/2026-10-19-p1b/03/pw/start-daemon.sh
~/code/enth-scratch/2026-10-19-p1b/03/pw/run-jack.sh ~/apps/reaper-7.82/reaper_linux_x86_64/REAPER/reaper
#    In REAPER: Preferences → Audio → Device → JACK. 8 tracks of enth (rt-stats bundle), each looping a chord;
#    track right-click → Track performance options → "Prevent anticipative FX" (= I_PERFFLAGS 2).
# 2. Per phase, 120 s, in a second terminal (phases: A closed, B one editor open idle, C one editor open while
#    playing, D covered, E minimised, F four editors open while playing; A and C repeated at the end):
P=A; pidstat -t -p "$(pgrep -x reaper)" 1 120 > $P-cpu.txt & nvidia-smi pmon -s um -d 1 -c 120 > $P-gpu.txt &
timeout 120 pw-top -b -r enth-rt-test > $P-pw.txt; wait
# 3. Afterwards
pkill -f "^pipewire -c .*enth-rt-test[.]conf"
```

- **Read per phase:**
  - the `enth-gui` thread's CPU % (thresholds: below 1 % idle, below 5 % animating);
  - `pmon` SM % and FB MB for REAPER (below 5 %, below 100 MB per editor);
  - the plugin's rt-stats p99.9 and `pw-top` ERR for REAPER's node, open against closed;
  - the egui frame log: CPU per frame p99 below 16.7 ms, about 30 fps achieved.
- **Then one short real-device run, to listen for clicks:** `pw-metadata -n settings 0 clock.force-quantum 64`, `pw-jack` REAPER on the normal daemon, phases A and C only, 10 minutes each. Set `clock.force-quantum 0` afterwards. Expect machine-caused xruns at 64 frames (the stutter config): only the open-against-closed difference counts.
- Update owner checklist C3 to match: anticipative FX off on the plugin tracks, interleaved phases, `pw-top -b` logged to a file.

## Plan check

| Doc says | Finding | Proposed change |
| --- | --- | --- |
| phase-1a.md and roadmap G1: "p99 `process()` under 25 % of the 64-sample deadline, in the test host (P1b)" | Reachable: engine p99 125–146 µs, about 10 % (RR, paced, loaded laptop). Not stated: scheduling class, warm-up, swaps | Add RR via rtkit, the 120 s length, p99.9 alongside, and the swap-block and crossfade-block conditions (Recommendations) |
| phase-1a.md open check: "During the 256-frame crossfade the CPU doubles, so the steady state must stay under about 12 %" | The doubling holds (1.6–2.0×), but the **swap block's state copy** is the real outlier: 0.8–1.06 ms paced. The model misses it | Replace with the measured swap-block numbers and the move fix |
| engine.md: "A surviving reverb copies its delay lines (about 1.6 MB at 48 kHz) on the audio thread … P1b should time it" | Timed: +140–210 µs pinned, up to +950 µs paced | Record the numbers; adopt the move (and pre-effects crossfade if accepted); a "moves, not copies" test |
| engine.md: "an equal-gain crossfade of the two plans' outputs" | Prototype 2 blends voice sums before one effect chain; unrelated edits stay bit-identical | Owner or coordinator decision; if accepted, update engine.md and design.md §3 |
| benchmarks.md: "p99.9 / max `process()` < 25 % of the deadline" | p99.9 passes under RR (166–185 µs); max (1.1–1.4 ms RR, 5–21 ms SCHED_OTHER) is the machine | Gate p99 and p99.9 under RR; report max with wall − cpu attribution; note `IRQ_TIME_ACCOUNTING` is off |
| benchmarks.md: "Plan swap time: swap without audible glitch; compile off-thread" | Compile 0.9 ms (G1) on the main thread is fine; the swap block is not, until fixed | Add the swap-block threshold |
| benchmarks.md xrun row and roadmap G0 "no GUI-caused xruns" (pending owner, Dummy Audio only) | Agent tier is possible: `process-stats --editor` A/B (RR: no measurable effect; SCHED_OTHER: misses double) and the private PipeWire daemon | Add the agent-tier proxy; keep the owner tier for GPU-driver effects |
| benchmarks.md procedure 1: "Headless REAPER under Xvfb (Dummy Audio) … 8 instances, held chord" | With anticipative FX on, `process()` is not on the device deadline | Set `I_PERFFLAGS = 2` on the plugin tracks; measure with the plugin-side histogram |
| owner-checklist C3: `pw-metadata … clock.force-quantum 64` and "if not, also … min-quantum 64" | Documented: settings bypass config limits, so force-quantum alone should work. Anticipative FX not mentioned. The private daemon avoids touching the owner's audio | Edit C3 as in the owner-attended plan above |
| AGENTS.md: "plugin debug builds also enable `assert_process_allocs`"; nightly RTSan over `enth-engine` and `enth-render` | Confirmed: it covers nice-plug's whole wrapper, debug only. RTSan can now cover the plugin through the test host | Add the RTSan test-host job (Recommendation 6) |
| benchmarks.md tools: "`perf` … blocked while `kernel.perf_event_paranoid` is 4" | Still 4 | Keep `CLOCK_THREAD_CPUTIME_ID` and rusage as the in-process substitutes (used here) |
| AGENTS.md parallel work: at most 2 heavy builds | Other researchers kept the load at 7–26 during these timings | Timings carry the load average; re-run the G1 numbers on a quiet machine before the gate |

## Gaps

- **No `enth-plugin` yet.** p99 numbers are engine-only (`rtbench`). `process-stats` was exercised on spike-ui-plugin (toy synth, `process()` p50 7–12 µs). The nice-plug wrapper's share for the real plugin is unmeasured; it is likely under 10 µs.
- **Loaded machine.** Every run had load 7–26. Repeat the G1 numbers on a quiet machine with `uptime` and the governor recorded.
- **REAPER:**
  - the JACK setting under Xvfb (the `linux_audio_mode` value) was not found;
  - whether Dummy Audio is paced, and at what thread priority, was not checked;
  - anticipative FX in 7.82 is documented only through the API string, a 2011 magazine article and forum posts.
- **Unexplained 10–35 ms lateness** on four of eight RR threads in the 16 × 8 run. Suspects: equal-priority RR contention with another application's RR 10 threads (the rtkit journal showed one acquiring RR 10 every 15 s), and the 25-thread rtkit cap. Not isolated.
- **Pre-effects crossfade:** no listening check. The effect of blending before the compressor on loud edits is not analysed beyond the tests.
- **RTSan via the host:** relies on the internal `__rtsan_realtime_enter`. Contended locks and syscalls inside a real nice-plug bundle were not tried, nor a full plugin build on nightly. A `thread::sleep` in the test cdylib was not reported.
- **Dummy-Driver semantics:** overruns slow the graph rather than raising JACK xrun callbacks. Real-device xruns (driver, interrupts) can only be seen on hardware: owner tier.
- **Swap cost under memory pressure** (14.5 GB of swap in use) and with many instances swapping together: not measured. `mlockall` was tried only in the single-thread swap loop (no change).
- **GPU numbers:** owner tier, not run.
- The throwaway REAPER copy in `reaper-sb/` (233 MB) can be deleted.
