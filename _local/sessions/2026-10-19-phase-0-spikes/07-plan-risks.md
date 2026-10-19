# 07 — Plan risks and unknown unknowns

- **Date:** 2026-10-19
- **Sub-researcher:** 07 (plan risks, unknown unknowns, parallel Phase 0 schedule)
- **Questions answered:** all seven in the brief: (1) the ten riskiest assumptions; (2) prior art and what slowed it; (3) the shape of Phase 0 and a parallel schedule; (4) the Recreate metric gate and how trainers were received; (5) evidence for the agent loop and the G1 gate; (6) unknown unknowns; (7) changes to the roadmap and Phase 0.
- **Method:** I read `brief.md`, every file in `docs/` and `docs/adr/`, and the 2026-10-18 effects `summary.md`. I then used web search and fetch, `gh api`, and the crates.io, Codeberg and Launchpad APIs, and ran read-only commands on the owner's machine (`mount`, `lsblk`, `free`, `swapon`, `dpkg`, `apt-cache policy`, `prime-select`, `ulimit`, `/proc/driver/nvidia/version`). Nothing was built, installed or changed.
- **Labels:** *measured* means local command output, or numbers read from an API or a paper's table. *Documented* means official docs, release pages, papers or READMEs. *Anecdotal* means forums, blogs or vendor marketing. *Computed* means my own arithmetic.

## Answer in brief

- **The riskiest assumption is the test bench, not the code.** The docs assume "Linux + REAPER" with "Bitwig and/or Ardour" as second hosts. In fact:
  - The owner has only the **Windows** REAPER, running in Bottles. A Windows process can't load a Linux `.clap`.
  - **Ardour does not host CLAP.**
  - Bitwig is paid.
  - The fix costs about 30 minutes of the owner's time and needs no code. Native REAPER 7.82 (2026-10-04) installs into the home folder from a tarball, without sudo. Carla 2.5.8 and Qtractor 0.9.39 are both in apt today and both host CLAP.
- **The machine is the second risk** (measured today):
  - The repo is on **NTFS through ntfs-3g (FUSE), on a USB-attached QLC SSD**.
  - Only **1.8 GiB of 15 GiB RAM is available, with 15 GiB of swap in use**.
  - The NVIDIA module and its userspace libraries are mismatched.
  - `ulimit -r` is 0, and `pipewire-jack` is not installed.
  - Together, these slow every agent build loop and limit parallel work to about two heavy builds at once. They also turn xrun and CPU figures into noise unless measurements run on a rebooted, quiet machine.
- **Phase 0 should run as parallel lanes after a ½–1 day skeleton.** The shell, engine/determinism, metric, egui panel and test-host lanes can run side by side, with UI embedding after the shell. That makes about **9–10 working days instead of 15–17**, plus about 6–8 hours of the owner's time at four points.
- **Cut, shrink and add:**
  - Cut the vizia and WebView arms; run vizia only if egui fails a threshold.
  - Shrink Task 5 and defer the poly-mod check to S1.
  - Add a cross-platform check that each seed produces the same target, and a check that the plugin can play its own audition notes.
- **The G0 "metric agrees with your ears" gate cannot pick between three metrics as written.** With 30 single-rater pairs, an observed ρ = 0.6 has a 95% CI of 0.31–0.79. Telling apart two metrics whose correlations differ by 0.15 needs about 70 or more judgements (computed). The fix has two parts:
  - Redesign the test as near-threshold triplets with repeats.
  - Decouple Recreate v0 from the audio metric. Syntorial and Synth Basics both shrink the problem (notched values; 2, 4 or any knobs) and give feedback per parameter.
- **Prior art says P1's 4–6 weeks is optimistic.** I found no AI-built Rust synth with a documented timeline.
  - Actuate went from repo creation to v1.0 in 83 days, and its author had already started five nih-plug plugin repos in the five months before (measured from GitHub).
  - OctaSine took about four years to reach 0.9, then stalled.
  - Meadowlark restarted and is still "currently incomplete".
  - So split P1, and measure Phase 0's actual effort against its estimates.
- **The G1 agent gate is risky in its validity, not its difficulty.** Evidence that LLMs can do sound design from numeric descriptors is thin. LLM2Fx and LLM2Fx-Tools cover effects, not synths. synth-rl and vital-mcp are 9–14 days old and publish no results. As written, the gate mostly tests the descriptors and hints, and it could pass trivially. Add 3 runs per goal and a blind human check, and run a half-day probe in Phase 0.
- **The roadmap has an ordering bug.** GR1 needs friends to play, but Windows and macOS builds come in S1 and the standalone app in S4. Ableton, Logic and Cubase still have no CLAP (July 2026). Ask the friends now which OS and DAW they use.
- **The docs miss two determinism traps.**
  - Rust documents its transcendental functions (`sin`, `exp`, `powf`…) as non-deterministic across platforms *and Rust versions*. Golden hashes will therefore break on toolchain bumps unless the DSP uses `libm` or its own approximations.
  - A randomizer whose accept/reject step depends on a render breaks "a challenge is just a seed" across platforms.
- **nice-plug is very young and moves fast:**
  - Its first crate was published on 2026-06-04, and it went from 0.1 to 0.4.2 in 14 weeks.
  - All of its 10 latest commits are by one maintainer.
  - An egui 0.37 and baseview 0.4.1 update merged on 2026-10-18 and is not yet released.
  - Pin a crates.io release rather than a Codeberg git dependency. Codeberg suffered DDoS outages in Dec 2025 and Jul 2026.

## Findings

### Q1 — The ten riskiest assumptions, ranked

The ranking weighs likelihood × impact on the next 3 months. "Cheapest retirement" is the smallest action that removes or measures the risk.

| # | Assumption (where) | Evidence against it | Impact | Cheapest retirement |
|---|---|---|---|---|
| 1 | **"Main test host: REAPER on Linux; second host Bitwig and/or Ardour"** (README; phase-0 Tasks 1, 3, 4; benchmarks) | The brief says the owner runs REAPER 7.80 only under Wine/Bottles, with no native Linux REAPER, Bitwig or Ardour. Wine runs Windows binaries: yabridge only bridges *Windows plugins into Linux hosts*, and nothing goes the other way ([yabridge README](https://cdn.jsdelivr.net/gh/robbert-vdh/yabridge@master/README.md), documented). Ardour maintainers said "no plans to add CLAP support" ([tracker #9417](https://tracker.ardour.org/view.php?id=9417), last updated 2024-08); in Nov 2024 they would only merge a patch ([LibreArts](https://librearts.org/2024/11/clap-api-two-years-later/)). Ardour is absent from the [clapdb host list](https://clapdb.tech/category/hostsdaws) (fetched 2026-10-19), and coverage of Ardour 9.0 (Feb 2026) doesn't mention CLAP (documented). After its 30-day trial, Bitwig disables save and export ([Bitwig](https://bitwig.com/support/shop_license_activation/48), documented). | Task 3 must-haves a, c and h, Task 4's whole measurement protocol and Gate G0 can't be run at all | **Owner, about 30 min:** install the native REAPER 7.82 tarball as a user, with no sudo ([download page](https://www.reaper.fm/download.php), 7.82 dated 2026-10-04; user-level install script per the [Cockos wiki](https://www.cockos.com/wiki/index.php/REAPER_for_Linux)). Then `sudo apt install carla` (2.5.8) or `qtractor` (0.9.39), both above clapdb's first CLAP versions (2.5.7 and 0.9.27); measured with `apt-cache policy` on 2026-10-19 |
| 2 | **The dev laptop can host agent loops, parallel work and trustworthy real-time measurements** (benchmarks; phase-0) | Measured 2026-10-19: `/media/vboxuser/G-samsung` is `fuseblk` (NTFS via `mount.ntfs-3g`) on `sdb`, a USB-attached Samsung 870 QVO (a QLC SSD). `free -h` showed 1.8 GiB available and 15 GiB of swap in use, with one `python` process at 5.6 GiB. `/proc/driver/nvidia/version` shows the open kernel module 595.91.07, while the brief reports NVML 595.99. `ulimit -r` is 0; the user is not in `audio`; `pipewire-jack` is not installed; and the kernel cmdline has no `preempt=` setting. | Slow incremental builds, the core of the owner's "iteration speed first" goal. Swap storms when two agents compile at once. Page faults on the audio thread produce xruns regardless of the GUI, so "0 GUI-caused xruns" is unmeasurable as specified | **Owner, about 1 h:** reboot; move the repo (or at least `CARGO_TARGET_DIR`) to the ext4 NVMe `/home` (359 GiB free, measured); `apt install pipewire-jack`; measure with heavy apps closed. Optional: `preempt=full threadirqs`, as Ubuntu Studio 24.04.3 does ([Ubuntu Studio](https://ubuntustudio.org/2025/08/ubuntu-studio-24-04-3-released/); [Ubuntu kernel tuning](https://discourse.ubuntu.com/t/fine-tuning-the-ubuntu-24-04-kernel-for-low-latency-throughput-and-power-efficiency/44834), documented). Sub-researcher 06 should supply the build-time numbers |
| 3 | **"The metric agrees with your ears on most pairs"** decides `metric v1` (roadmap G0; Task 6) | Computed with Fisher z: n = 30 gives a 95% CI for ρ = 0.6 of [0.31, 0.79], and the critical r at α = .05 is 0.36. Separating two metrics at ρ = 0.60 vs 0.75 (inter-metric r = 0.8) needs about 72 judgements; 0.60 vs 0.70 needs about 170 (Steiger approximation). Pairs "at varying distances" restrict nothing, so all three metrics will look good on the easy pairs. The literature finds no single best loss: the best loss depends on the scenario, and partial parameter distance agreed with blind listening in 5 of 7 scenarios ([Salimi et al., 2026-08-27](https://arxiv.org/abs/2608.27698), documented). Spectral distances have a poor sense of pitch direction ([Turian & Henry, 2020](https://arxiv.org/abs/2012.04572), documented). INSTRUMENTAL, which the docs call "our permanent baseline", "evaluate[s] on a single target sound", ran no listening test, and plateaus after about 10K evaluations ([arXiv 2603.15905](https://arxiv.org/html/2603.15905), documented) | The gate can't fail usefully, and `metric v1` ends up chosen by taste. If P1 and R1 are blocked on it, the schedule slips | **Design change, no extra cost:** use triplets ("which of A/B is closer to T?"), stratified toward the pass threshold; take 60–90 judgements over two sessions of 30 minutes or less, with 10 repeats to measure the owner's own consistency (details in Q4). Score Recreate v0 by structure on stepped parameters, so the game doesn't depend on the audio metric |
| 4 | **P1 (the Syntorial-sized synth, effects, editor, 72 host params, CLI v0 with 13 commands, goldens on 3 OSes + wasm32, agent gate) fits in 4–6 weeks** (roadmap) | Actuate (GPL-3.0, nih-plug + egui): repo created 2023-08-22, v1.0.0 on 2023-11-13, i.e. 83 days. Its author had created five nih-plug plugin repos in the previous five months ([GitHub](https://github.com/ardura/Actuate/releases); measured via `gh api`). OctaSine: created 2019-07, v0.5 in 2021-02, v0.9 with CLAP in 2023-08, last push 2024-07 (measured). METR's 2025 RCT found experienced open-source developers 19% *slower* with AI ([METR, 2025-07-10](https://metr.org/blog/2025-07-10-early-2025-ai-experienced-os-dev-study/), documented). METR's 2026 follow-up calls its own data "an unreliable signal" ([METR, 2026-02-24](https://metr.org/blog/2026-02-24-uplift-update/), documented). Every effort figure in the docs is unvalidated (effects summary, open check 15) | The headline hypothesis (is Recreate fun and fair?) is first tested about 3 months in, and the schedule pressure lands on the riskiest part (editor and host integration) | Record actual effort per Phase 0 lane against its estimate in `docs/spikes/summary.md`: that gives the first real calibration. Split P1 into a headless P1a and a plugin/editor P1b (see Plan check) |
| 5 | **nice-plug is a stable enough base; churn costs under a day a month** (ADR-0001) | crates.io (measured 2026-10-19): first version 0.1.x on 2026-06-04; 0.2.0 on 07-24, 0.3.0 on 08-17, 0.4.0 on 09-09, 0.4.2 on 09-14. That is four breaking minor bumps in 14 weeks and 15,687 downloads in total. On Codeberg, the 10 latest commits are all by Billy Messenger, including "update to egui 0.37.0" (2026-10-17) and "update to baseview 0.4.1" (2026-10-18), not yet released (measured via the Codeberg API). The repo has 79 stars. The same maintainer also runs Firewheel (last push 2026-09-05) and Meadowlark. Codeberg was DDoSed in Dec 2025 ([HN, 2025-12-02](https://hn.nuxt.dev/item/46131693)) and had traffic-spike outages on 2026-07-31 ([StatusGator](https://statusgator.com/services/codeberg/map)) (anecdotal/aggregator) | Spike results go stale within weeks. A git dependency on Codeberg can break CI. The project inherits a bus factor of 1 | Run the shell spike on the newest crates.io release (0.4.2, or the egui 0.37 release if one appears by then). Depend through crates.io, never `git =`. Budget one upgrade day a month. Keep clack as the fallback (sub-researcher 01 has the details) |
| 6 | **Recreate (R1) can be validated with friends before Track S** (roadmap GR1: "you and friends play 20 rounds each") | Windows/macOS builds are S1 and the standalone app is S4. CLAP is not in Ableton Live, Logic, Pro Tools or Cubase ([Antares, 2026-07-27](https://www.antarestech.com/blog/vst3-au-aax-clap-plugin-formats-2026); [Ableton forum request](https://forum.ableton.com/viewtopic.php?p=1802901)). The same Antares article wrongly lists FL Studio as non-CLAP: Image-Line documents CLAP since 24.1.1 ([manual](https://www.image-line.com/fl-studio-learning/fl-studio-online-manual/html/plugins_supported.htm)), and clapdb lists Studio One Pro 7 (documented; the sources conflict). Unsigned macOS plugins hit Gatekeeper, and testers report "Open Anyway" not working for VST3 ([JUCE forum](https://forum.juce.com/t/mac-open-anyway-not-working-for-unsigned-mac-vst3/64574), anecdotal) | GR1 can't be met, or forces S1 work into R1 | **Owner, 5 min:** ask the friends which OS and DAW they use. If most are on Windows/macOS with Ableton or Logic, add to R1 a standalone Recreate build (nice-plug ships a standalone wrapper, per ADR-0001) and a Windows VST3 build |
| 7 | **"A challenge is just a seed + settings"; same-platform hashes are bit-exact** (design §5, §6, §9; R5) | Rust's docs: the precision of `sin`, `exp`, `powf`, `tanh`… "is non-deterministic. This means it varies by platform, Rust version, and can even differ within the same execution" ([f32 docs](https://doc.rust-lang.org/std/primitive.f32.html), documented). Design §9's "musical/safe" generator accepts a target only if it is audible, not clipping, and differs audibly from the start. Those are *render-dependent* branches, so a 1-ulp difference can flip accept/reject and produce different targets from the same seed on two machines (inference) | Duel codes and lesson targets diverge across machines; golden hashes churn on every toolchain bump | In Task 5, generate 1,000 targets from seeds on Linux, Windows (CI) and wasm32 and compare *patch* hashes; render the same patch with `std` maths and with the `libm` crate. Then make target generation parameter-space only, or store the accepted attempt index in the challenge code |
| 8 | **"An agent solves at least half of 10 starter text goals within 20 renders each"** is a meaningful G1 test (roadmap; agent-cli) | No published benchmark covers LLMs reaching descriptor targets on a synth. The closest are: LLM2Fx (effects only); LLM2Fx-Tools, where Gemini 2.5 Flash scores 78% effect accuracy and MUSHRA 56.5 against a hidden reference at 81.5 ([arXiv 2512.01559](https://arxiv.org/html/2512.01559v2)); and synth-rl, created 2026-10-05, which says "Current models are not good at this" and "edit tasks can largely be solved from text alone" when parameter values are shown, but publishes no numbers ([synth-rl](https://github.com/luwux/synth-rl)). If the expectation lists are given, the loop is constraint satisfaction driven by measured hints: it tests the *descriptors* (envelope, sweep and beating detection, all in-house) more than the agent | The gate passes trivially without the sounds matching the text, or fails because of descriptor bugs. Neither result says whether agents make good sounds | A half-day probe in Phase 0 on the metric spike's renderer (lane P). At G1: 3 runs per goal; name and date the model; a blind human check that the results sound like their text |
| 9 | **The UI spike builds two toolkits, a CLAP test host and the full protocol in 4–5 days** (Task 4) | The decision rule already favours egui: vizia wins only if it is ≥ 30% cheaper *and* its lesson pane is clearly better. The owner wants fast AI iteration, and egui has the larger code corpus (docs). Self-drawn UIs (egui, vizia, iced) don't tell hosts that a text field has focus, so REAPER keeps keystrokes for itself ([nih-plug #219](https://github.com/robbert-vdh/nih-plug/issues/219), documented). Actuate's README tells Bitwig users to type a space with shift+space, and REAPER users to enable "send all keyboard input to plugin" ([README](https://github.com/ardura/Actuate)); it also had "File Dialog not working in Bitwig/Linux" (issue, 2023-11-15) (documented) | Task 4 overruns. Host-integration work keeps surprising, and the checklist fails on keyboard items for reasons unrelated to the toolkit | Build only the egui arm, and treat vizia as contingent. Split Task 4 into U1 (standalone panel), H (test host) and U2 (embed + measure). Expect the keyboard items to fail, log them, and decide the policy in S1 |
| 10 | **A full research session before every milestone, plus parallel agents, fits one owner's review bandwidth** (research-process; AGENTS.md) | The last session produced sub-reports of 43, 55 and 40 KB and a 44 KB summary (measured with `ls -la`). Practitioners put the limit at reviewing, not git: three to five parallel sessions, with conflicts moved to merge time ([Steve Kinney](https://stevekinney.com/courses/ai-development-setup/worktrees), anecdotal) | The owner becomes the bottleneck; lanes idle waiting for review; summaries get skimmed | Scale sessions to milestone risk (2–3 axes for small milestones). Cap `summary.md` at about 2 pages plus a decision table. Give each lane a machine-readable result (JSON plus a short table) and review in one daily slot |

Just outside the top ten: a corpus for the "musical" randomizer doesn't exist yet (Q6). Plugin IDs must be fixed before friends save projects (Q6). Agents may reproduce GPL DSP from memory (Q6).

### Q2 — Prior art: solo and small-team Rust synths and AI-assisted audio projects

| Project | Team / licence | Timeline (measured via `gh api` / crates.io unless noted) | What slowed or stopped it |
|---|---|---|---|
| [Actuate](https://github.com/ardura/Actuate) (synth, sampler, granulizer; nih-plug + egui) | Solo (Ardura); GPL-3.0 | Repo 2023-08-22 → v1.0.0 2023-11-13 (**83 days**) → v1.4.5 2026-04-15; last push 2026-06-04. The author's earlier nih-plug repos: Subhoofer (2023-03-24), Duro-Console, Scrollscope, Canopy-Reverb, GladeDesk | Host integration on Linux. README notes on keyboard input in Bitwig and REAPER; issues "File Dialog not working in Bitwig/Linux", "No sound in bitwig demo w/ linux" (2023-11-15), "Linux - Inconsistent MIDI behavior" (2024-02-05), "Crash in Reaper when clicking on a parameter description" (2024-12-29). The README says "still a work in progress/alpha" (documented) |
| [OctaSine](https://github.com/greatest-ape/OctaSine) (4-operator FM) | Solo; AGPL-3.0 | Created 2019-07-20; v0.5.0 2021-02-14; v0.9 with CLAP 2023-08; last push 2024-07-03 | About four years to 0.9, then quiet for more than two years. Linux is X11-only (documented, per indexers) |
| [Meadowlark](https://codeberg.org/Meadowlark/Meadowlark) (DAW) | Small team led by BillyDM; AGPL-3.0 | GitHub 2021-06-16 → moved to Codeberg 2025-09-22; the description still says "(currently incomplete)"; old code kept in `old` branches | Infrastructure first: engine, GUI and plugin plumbing were rebuilt several times. Its lead now also maintains nice-plug and Firewheel (documented by README; the reason for the restarts is my inference) |
| HexoSynth / synfx-dsp (modular) | Solo; GPL | Listed in docs/research.md; the effects session found its GitHub repo no longer publicly reachable | Not verifiable today. It built its own GUI toolkit; no conclusion drawn |
| [suletta](https://github.com/hexeaktivitat/suletta) (subtractive) | Solo learning project; GPL-3.0 | 2022-10 → last push 2024-04; the todo list still includes oscillator and filter stages | Abandoned before a usable synth |
| [culsynth](https://github.com/rbmj/culsynth) | Solo; MIT; 3 stars | 2023-10 → 2026-01 | "Only VST3 is being tested" (documented) |
| [web-synth](https://github.com/Ameobea/web-synth) | Solo; Rust DSP in wasm | 2018-11 → still pushed 2026-08-13 | A survivor: an eight-year solo project, shipped as a website from the start |

**AI-assisted audio projects (2025–2026).** No public Rust/CLAP synth built mainly with coding agents documents its timeline (searched 2026-10-19).

- *Sound On Sound* (Jan 2026) built a JSFX tool with Claude. Testing was manual inside REAPER, fixes introduced new bugs, and one feature "didn't work, even after Claude tried many times" ([SOS](https://www.soundonsound.com/techniques/vibe-coding-how-anyone-can-create-plug-ins), anecdotal).
- A KVR thread (8–26 Apr 2026) centres on distrust and "sameness" of vibe-coded plugins, and doubts that complex synth DSP works this way ([KVR](https://www.kvraudio.com/forum/viewtopic.php?p=9234441), anecdotal).
- A vendor selling nih-plug prompt packs claims unguided AI output aliases, spikes CPU through denormals and allocates on the audio thread ([AudioMage](https://www.audiomage.com/products/clap-plugin-forge/), vendor claim).
- The AI-driven synth tools are days old: [synth-rl](https://github.com/luwux/synth-rl) (created 2026-10-05, 0 stars, GPL-3.0) and [vital-mcp](https://github.com/mard4/vital-mcp) (2026-10-10, 2 stars, GPL-3.0) (measured).

**Patterns that apply to enth:**

1. Building infrastructure before a playable instrument (own toolkit, own engine framework, DAW-scale plumbing) is the common way these projects stall.
2. Host integration on Linux (keyboard focus, file dialogs, MIDI quirks, per-host behaviour) eats the time that DSP doesn't.
3. The first usable synth took an experienced solo developer about 3 months, after about 5 months of practice on the framework.
4. Bus factor 1 everywhere, including the shell enth depends on.

The docs already push back on (1) with thin shells, spikes and "don't build yet". The roadmap does not yet budget for (2), and P1's estimate ignores (3).

### Q3 — Is Phase 0's shape right for AI-agent development?

**No. The sequence ("do the tasks in order… stop and wait") is shaped for one agent and one reviewer.** The actual dependencies are few:

- **Must exist first (serial):**
  - The owner's machine setup (hosts, reboot, ext4, sudo packages, remote repo).
  - The workspace skeleton (Task 2), with the `spikes/*` and `tools/*` member globs present *from day one*, so lanes never edit the root `Cargo.toml`.
  - A pinned nice-plug release and a pinned egui version, which the egui panel lane must match.
  - Permanent plugin IDs.
- **Independent of each other** once the skeleton exists:
  - The shell (Task 3).
  - Engine and determinism (Task 5).
  - Metric and CMA-ES (Task 6), with its own throwaway renderer.
  - An egui reference panel in a standalone window.
  - The clack-host test host, which can use nice-plug's example plugin until the shell bundle exists.
- **Dependent:** UI embedding and measurement (needs the shell, the panel and the test host); closing Phase 0 (needs everything).

**Merge, cut, shrink, defer:**

| Item | Verdict | Why |
|---|---|---|
| Task 1 (machine setup) | **Merge** into a day-0 owner checklist plus an agent-run `xtask doctor` that writes `environment.md` | Most of it needs sudo or a reboot; verifying it is mechanical |
| Task 3 must-have (d), poly-mod | **Defer to S1** | Bitwig is the only reference host and isn't installed; (d) doesn't force a shell switch (ADR-0001) |
| Task 4 vizia arm | **Cut, unless egui fails a threshold** | The decision rule favours egui; AI familiarity; saves about 2–3 days |
| Task 4 WebView arm | **Cut** | The owner's strong preference is native, non-WebView |
| Task 4 protocol | **Shrink** to the decision subset: closed / idle / animating CPU, PSS per editor, growth over 200 open/close cycles, open time, xruns with the editor open vs closed (A/B). Defer watts and 50-instance scaling to S1 | They don't change the choice; S1 re-measures the real editor anyway (GS1) |
| Task 4 test host | **Shrink and run in parallel** (lane H); fall back to REAPER-only numbers if it slips | Reused later for soak and CI |
| Task 5 | **Shrink to 1–2 days, but add** a seed → target hash check across platforms, a `std` vs `libm` comparison, and a P1-like patch for speed (3 oscillators, filter, 2 envelopes, LFO, 8 voices, plus a reverb stand-in) | 100× realtime for one osc→filter→amp voice is close to certain to pass (inference); design §7's solver budget depends on realistic patches, and reverb sits in the scored path |
| Task 6 listening | **Redesign** (Q4) | A 30-pair single-rater test can't separate the metrics |
| New, optional: lane P | **Add**: a half-day agent-loop probe | Retires risk #8 about four weeks earlier, at no cost to the critical path |

**What needs the owner:** sudo installs, the reboot and moving to ext4 (day 0, about 2 h); creating the remote repo and the first push (5 min); host checks in REAPER and Carla/Qtractor: automate, save and reload, slot rename (about 1–1.5 h, lane A); two listening sessions of 30 minutes or less (lane C); the host checklist items that need hands and eyes, such as HiDPI, keyboard and drag (about 1 h, lane U2); approving ADRs at close.

**What agents do unattended:** the skeleton, xtask, CI, all spike code, the test host, scripted measurements (`pidstat`, `smem`, `perf` wrapped in `xtask` commands that emit JSON), CMA-ES runs, and the reports.

**Constraints on parallelism, measured or documented:**

- At most two compile-heavy lanes at once with today's RAM. The heavy lanes pull nice-plug, baseview, egui and clack-host (A, U1, H, U2); B and C have small dependency trees.
- Concurrent builds in different worktrees collide over a shared `target/` and `.cargo-lock` unless each has its own target path ([Rust forum](https://users.rust-lang.org/t/announcements-cargo-worktree-fix-build-isolation-in-git-worktrees/139192), anecdotal). Put per-worktree target dirs on ext4.
- Expected merge hotspots: `Cargo.lock` (regenerate), `xtask/` (assign `bundle`/`install` to lane A only), `deny.toml`. Shared registration blocks were the main conflict source in one ten-agent postmortem ([Bernstein](https://bernstein.readthedocs.io/en/latest/blog/ten-agents-one-release/), anecdotal).
- Each lane writes only its own `docs/spikes/<lane>.md`.

The schedule table is in its own section below.

### Q4 — Recreate and sound matching: is "the metric agrees with your ears" realistic?

**As written, it is realistic to pass and not informative.** With pairs that span "varying distances", any reasonable distance tracks gross differences, so all three candidates will "agree on most pairs". The decisions that matter are near the pass threshold: is this close enough, and which of two near-misses is closer? Random pairs barely sample those.

- **Statistics (computed).** For n = 30, an observed ρ = 0.6 has a 95% CI of 0.31–0.79; at n = 60 it is 0.41–0.74; at n = 90, 0.45–0.72. Distinguishing ρ = 0.60 from 0.75 between two metrics that correlate 0.8 with each other needs about 72 judgements at 80% power (Steiger's test, approximate).
- **Literature (documented).**
  - The best loss depends on the scenario: SIMSE best for filter cutoff, DTW-envelope for AM, JTFS for pitch glides; partial parameter distance agreed with listening in 5 of 7 scenarios ([Salimi 2026](https://arxiv.org/abs/2608.27698)).
  - Spectral losses misjudge pitch direction ([Turian & Henry 2020](https://arxiv.org/abs/2012.04572)).
  - INSTRUMENTAL's composite loss was validated on one target, without a listening test ([arXiv 2603.15905](https://arxiv.org/html/2603.15905)).
  - So the Phase 0 metric should be called *provisional v1*, recalibrated with friends' ratings in R1.
- **Speed is not the risk (computed).** At 100× realtime, a 2 s render takes 20 ms, i.e. about 3,000 renders per core-minute. INSTRUMENTAL's loss plateaued after about 10K evaluations on 28 parameters. A 10-parameter template within "under a minute on 8 threads" (about 24K renders) is plausible. The risks are local minima and the metric.

**A better listening design (same owner time, about 2 × 25 min):**

1. Triplets: the target T and candidates A and B; "which is closer to T?". This is faster, less fatiguing and more sensitive than 1–5 ratings.
2. Two-thirds of the triplets near the pass threshold, one third easy anchors.
3. 10 repeated triplets, to measure the owner's own consistency. Report metric agreement only where the owner is consistent.
4. Loudness-matched, randomised, blind to the metric. Two sessions; ear fatigue is a reported side effect of training ([Gearspace](https://gearspace.com/threads/when-do-you-train-your-ears.1431002/), anecdotal).
5. Gate: `metric v1` matches the owner's consistent choices in ≥ 75% of near-threshold triplets, and the margin between candidate metrics is reported with a CI.

**How existing trainers were received.** Reddit was blocked for my user agent, so first-hand player reviews are thin.

| Product | What players and reviewers liked | What they disliked / what the maker changed | Lesson for enth |
|---|---|---|---|
| Syntorial (Audible Genius) | Hidden-patch challenges recreated "using just your ears", with a closeness score; "well presented and fun"; Randomizer mode for endless practice ([MusicTech, 2014-02-11](https://musictech.com/reviews/audible-genius-syntorial-review/); [Best Service](https://www.bestservice.com/en/syntorial_2_0.html), documented) | Parameters are **notched** ("choose from, say, five different envelope attack times"). The Primer synth's 6-voice cap ([Keyboard via Plugin Boutique](https://www.pluginboutique.com/articles/531-Syntorial-Review-at-Keyboard-Mag-com)). Price. In 2.0, fixed values were "remapped for consistent progress and fewer frustrating roadblocks" (Best Service); Plugin Boutique's copy, as quoted in search results and not fetched, says "to avoid differences that are too subtle to hear", and that group challenges were cut from 6 patches to 3. 2.0 also added an orange "almost" score (documented, vendor copy) | Discrete steps, with each step audibly different, make ear matching feasible and scoring trivial (per-parameter green/orange/red). Long challenge chains frustrate |
| [Synth Basics sound match](https://synthbasics.com/sound-match) | Difficulty = 2, 4 or any knobs; a % match; a warmer/colder meter; a coach naming the furthest parameter (fetched 2026-10-19; undated) | No reviews found | Closest to enth's R1 design. The "coach names the worst knob" pattern is the sensitivity hint in design §7 |
| SoundGym | Gamified daily drills; a sense of progress; helped one user find problem frequencies faster ([Gearspace](https://gearspace.com/threads/when-do-you-train-your-ears.1431002/); [Future Music 2017](https://www.pressreader.com/australia/future-music-9629/20170112/283278867403113), anecdotal) | Subscription price ($24.95/month or $119/year per [SOS](https://www.soundonsound.com/node/4923945); KVR lists $14.95/month, so the sources conflict); ear fatigue; easy at first, then "more subtle" values | Plan R4 games as short drills; keep sessions short |
| [Tats (RBMA)](https://www.tinymixtapes.com/news/tatsuya-takahashi-releases-synth-game-red-bull-music-academy) | 60-second timed sound recreation | No data | A timer adds play value: the `time` scorer |

**Implication:** none of the trainers I found grades players purely on audio similarity. All of them constrain the problem: few free parameters, stepped values, feedback per parameter. For R1:

- Use stepped values and structure scoring at the lowest difficulties.
- Fade in the `sound` score's weight as the metric earns trust.
- Keep "differs audibly from start" as a parameter-space rule (a minimum step), not a render test (also see risk #7).

### Q5 — Agent CLI loop: evidence and the G1 risk

| Evidence (2025–2026) | What it shows | Type |
|---|---|---|
| LLM2Fx ([arXiv 2505.20770](https://arxiv.org/abs/2505.20770), WASPAA 2025; v2 2025-07-17) | Text LLMs predict EQ and reverb parameters zero-shot and beat earlier optimisation approaches; DSP features, code and few-shot examples in the prompt help. The abstract gives no numbers | Documented |
| LLM2Fx-Tools ([arXiv 2512.01559](https://arxiv.org/html/2512.01559v2), ICLR 2026) | A fine-tuned Qwen3-4B with an audio encoder: 80% effect accuracy and order correlation 0.56; Gemini 2.5 Flash 78% / 0.54. MUSHRA (16 listeners): hidden reference 81.5, LLM2Fx-Tools 62.8, Gemini 56.5. Limitations: one-to-many mappings and single-instrument sources | Measured (paper tables) |
| synth-rl ([GitHub](https://github.com/luwux/synth-rl), created 2026-10-05) | An environment for agents editing Vital through 36 curated controls, scored on rendered descriptors. States that current models "are not good at this", especially when the answer is only in the audio, and that edit tasks are "largely" solvable from text when values are shown. No results published | Anecdotal (README) |
| vital-mcp ([GitHub](https://github.com/mard4/vital-mcp), created 2026-10-10) | An MCP server; optional rendering through DawDreamer returns measurements so "the model can judge the result and fix it". No evaluation | Anecdotal (README) |
| SynthScribe ([arXiv 2312.04690](https://arxiv.org/abs/2312.04690)) | Multimodal retrieval plus a genetic algorithm, not an LLM; user study with 6 musicians | Documented |
| SonicBench (in docs) | Audio LLMs score near chance on basic attributes | Documented (docs) |

**Risk assessment for G1:**

- **Difficulty: low to moderate**, *if* the 10 goals ship with expectation lists and the hints are measured. The agent then mostly follows `check` hints within a known template, and 20 renders is generous for 3–5 expectations (inference).
- **Validity: high risk.**
  1. The oracle is our own descriptors, so a pass proves descriptor and goal consistency, not "sounds like a warm pad" (Goodhart).
  2. Descriptor extraction (attack and release estimates, `filter_sweep`, `detune.beating_hz`, `words`) is new in-house code with no permissive reference crate (see sub-researcher 04). Its bugs show up as agent failures.
  3. LLM runs are non-deterministic; one run per goal is noise.
  4. The model and its date aren't specified, so the baseline can't be compared over time.
  5. Cost and tokens per run aren't budgeted.
- **Cheapest retirement:**
  - Lane P in Phase 0: expose `describe`/`check` over the metric spike's renderer and let an agent attempt 5 text goals.
  - At G1, specify the protocol: given expectations, 3 runs per goal, median, model name and date, tokens. Add a blind human check that at least 7 of the 10 final sounds match their text.
  - Keep agent-written expectations as a separate, harder track.

### Q6 — Unknown unknowns

1. **Hosts.**
   - Native REAPER for Linux: 7.82, dated 2026-10-04 ([download](https://www.reaper.fm/download.php)); 60 days of fully functional evaluation ([purchase](https://www.reaper.fm/purchase.php)); can run from the extracted folder or install to the home folder (Cockos wiki, documented). Whether the owner's Windows licence covers Linux is not stated on the purchase page (gap).
   - Free Linux CLAP hosts: Carla and Qtractor in apt (measured); Zrythm and Anklang (clapdb).
   - Bitwig is the only poly-mod reference.
   - Check in Task 3: can the plugin play its own audition notes (Recreate's "performance") while the host transport is stopped, in REAPER and the second host? Hosts may stop calling `process` on idle instruments (unverified; cheap to test).
2. **Linux audio.**
   - Mint 22.3 ships PipeWire 1.0.5 (patched by Ubuntu in July 2026 for CVE-2026-14324/-14330, per [ubuntuupdates](https://ubuntuupdates.org/package/core/noble/main/updates/pipewire)); upstream is at 1.6.x, with 1.6.4 in May 2026 ([LinuxToday](https://www.linuxtoday.com/blog/pipewire-1-6-4-multimedia-framework-released-with-jack-bluetooth-and-alsa-fixes/)) (documented).
   - Without `pipewire-jack`, native REAPER can't use JACK on PipeWire, and 64-sample tests need it (`pw-jack reaper`, quantum 64). Measured: package not installed; candidate 1.0.5-1ubuntu3.3.
   - Measured: `ulimit -r` = 0, and rtkit is installed. PipeWire threads get RT through rtkit, but anything else wanting RT doesn't.
3. **GPU.**
   - Measured: `prime-select query` returns `nvidia`; the open kernel module 595.91.07 is loaded, while the brief reports userspace NVML 595.99. Reboot before any GPU number.
   - egui on baseview renders through OpenGL, so GPU and power numbers depend on the PRIME profile. Record it with every result.
4. **Storage and RAM.** See risk #2. Also expect slower `git status`, and inotify-based tools (rust-analyzer, watchers) to behave worse on FUSE than on ext4 (inference). Moving to `/home` (ext4, 359 GiB free, measured) is the simplest fix.
5. **CLAP adoption (2026).**
   - Hosts with CLAP: Bitwig, REAPER, FL Studio (since 24.1.1, 2024), Studio One Pro 7, Carla, Qtractor, Zrythm, MultitrackStudio, MuLab, n-Track ([clapdb](https://clapdb.tech/category/hostsdaws); [Image-Line](https://www.image-line.com/fl-studio-learning/fl-studio-online-manual/html/plugins_supported.htm)).
   - Without CLAP: Ableton, Logic, Pro Tools, Cubase, Ardour ([Antares, 2026-07-27](https://www.antarestech.com/blog/vst3-au-aax-clap-plugin-formats-2026); Ardour tracker).
   - VST3 is therefore needed for Ableton and Cubase friends, and AU (via clap-wrapper) for Logic friends.
6. **Naming and IDs.**
   - crates.io names `enth`, `enth-dsp`, `shuhari`, `slancio`, `yamabiko`, `transonic`, `risset`, `essor` and `prokope` were free on 2026-10-19; `mimesis`, `ferrite` and `headroom` are taken (measured via the crates.io API).
   - GitHub names `enth` (an organisation since 2014), `shuhari`, `slancio`, `yamabiko` and `transonic` are taken by users or orgs (measured).
   - No EUIPO or USPTO search was done (gap).
   - More urgent than the name: the CLAP descriptor `id` ("should be unique to your plugin… reverse URI", [clap/plugin.h](https://github.com/free-audio/clap/blob/main/include/clap/plugin.h)) and the VST3 class IDs are what hosts use to find the plugin when reopening projects. Choose name-independent permanent IDs in Task 3, before any friend saves a project (the per-host recall behaviour is inferred, not verified).
7. **Determinism across Rust versions** (risk #7). Golden hashes keyed only by platform will churn on `rust-toolchain.toml` bumps if the DSP calls `std` transcendental functions. Store the toolchain version next to the hashes, or use `libm`.
8. **No data for the "musical" randomizer.** Design §9 samples "from distributions fitted to good presets", but enth has no presets, and fitting other synths' factory presets raises licence questions. Use hand-written priors per parameter (ranges, log scales, forbidden combinations) for R1, and fit later from enth's own presets.
9. **Distributing to friends on macOS.** Unsigned bundles get quarantined; the workarounds use Terminal (`xattr`/`spctl`); "Open Anyway" reportedly doesn't work for unsigned VST3 ([JUCE forum](https://forum.juce.com/t/mac-open-anyway-not-working-for-unsigned-mac-vst3/64574); [HISE forum](https://forum.hise.audio/post/96762), anecdotal). Notarisation needs the Apple Developer Program, $99/year per those forum posts (Apple's current price not checked).
10. **Codeberg availability** (risk #5). Fetch nice-plug from crates.io; consider `cargo vendor` for CI.
11. **GPL regurgitation by coding agents.** Vital, Surge and Dexed are widely published GPL code. An agent asked for "a wavetable oscillator" may reproduce it from memory (inference, not documented). Mitigation: AGENTS.md should require DSP written from cited papers and notes in `docs/dsp/`, with review of any suspiciously idiomatic port.
12. **LLMs in CI** (`cargo xtask agent-eval`): this needs API access, costs money and is non-deterministic. Run it nightly or on demand, never as a merge gate.
13. **Owner bandwidth** (risk #10): the actual critical resource once agents run in parallel.
14. **Wayland.** Mint 23 with Wayland is expected around Christmas 2026 (research.md). baseview is X11-only, so stay on the X11 session through Phase 0–1 and re-run the host checklist under XWayland then.

### Q7 — What I'd change (blunt)

1. **Fix the bench before writing code.** Native REAPER, a second CLAP host that actually loads CLAP (Carla or Qtractor, not Ardour), `pipewire-jack`, a reboot, an ext4 working tree, and measurement hygiene.
2. **Run Phase 0 as parallel lanes after a one-day skeleton:** about 2 weeks of calendar time instead of 3–4, with about 6–8 hours of the owner's time.
3. **Drop vizia and WebView from Phase 0;** egui stays unless it fails a threshold.
4. **Replace the 30-pair rating test** with near-threshold triplets plus repeats, and make `metric v1` explicitly provisional. **Don't let it block P1.**
5. **Split P1:** P1a headless (params, patch, engine, render, analysis, CLI v0, Recreate core in the CLI, plus a throwaway playable standalone prototype) and P1b plugin and editor. The headline hypothesis then gets tested in weeks, not months, in the fastest loop agents have.
6. **Score Recreate v0 like Syntorial at low difficulty** (stepped values, per-parameter feedback); the audio score grows with difficulty.
7. **Make seeds cross-platform-safe now:** a parameter-space randomizer, and `libm` or toolchain-keyed goldens.
8. **Give G1's agent gate a protocol** (3 runs, named model, blind human check), and probe the loop in Phase 0.
9. **Plan friend builds for GR1:** ask the friends; standalone and VST3 if needed.
10. **Fix plugin IDs** before anyone saves a project.

## Proposed parallel Phase 0 schedule

Times are agent working days unless marked. "Heavy" lanes pull nice-plug, baseview, egui or clack-host; with today's RAM, run at most two heavy lanes at once.

| Task | Depends on | Can run alongside | Needs the owner? |
|---|---|---|---|
| **O — Owner setup** (½ day of owner time): reboot (clears the NVIDIA mismatch); move the repo, or at least `target/`, to ext4 `/home`; `sudo apt install` the Task 1 list + `pipewire-jack carla` (or `qtractor`) `hyperfine`; native REAPER 7.82 user install; clap-validator 0.4.1 Linux binary ([release 2026-07-22](https://github.com/free-audio/clap-validator/releases)); create the remote repo. Optional: `preempt=full threadirqs`, RT limits | — | T0 | **Yes:** sudo, reboot, accounts (~2 h) |
| **T0 — Research → summary → doc edits** (1 d) | — | O | Yes: read the summary, approve the edits |
| **S — Skeleton + CI** (Task 2, ½–1 d). `members = ["crates/*","spikes/*","tools/*","xtask"]` from day one; `xtask doctor` checks hosts, targets, tools, filesystem type, free RAM/swap, PRIME mode, `ulimit -r` and the PipeWire quantum, and writes `docs/spikes/environment.md`. Pin the nice-plug and egui versions for all lanes; choose permanent CLAP/VST3 IDs | T0 (only the edits touching Task 2); O for the push | Nothing: every lane branches from it | Yes, 5 min: push |
| **A — Shell spike** (Task 3, 3–4 d, heavy). Must-haves a, b, c, e, f, g, h; plus self-triggered audition notes with the host stopped, versioned state blob, permanent IDs. (d) poly-mod moves to S1 | S; O (hosts) | B, C, U1, H | **Yes:** REAPER and Carla/Qtractor checks (~1–1.5 h) |
| **B — Engine + determinism** (Task 5, 1–2 d, light). ×realtime for 1 voice and for a P1-like 8-voice patch with a reverb stand-in; same-platform hash; wasm32 under wasmtime and Windows CI feature diff; 1,000-seed → patch-hash test across platforms; `std` vs `libm` | S (the remote for Windows CI) | A, C, U1, H | No |
| **C — Metric + CMA-ES** (Task 6, 2–3 d, light). Own throwaway 10-parameter renderer; features; three distances; CMA-ES on 20 targets; triplet listening script with repeats | S | A, B, U1, H | **Yes:** 2 listening sessions of ≤ 30 min |
| **U1 — egui reference panel as a standalone dev window** (2 d, heavy-ish). 64 knobs, 16-row matrix, draggable cards, modulation rings, scopes fed by a fake lock-free ring, lesson pane, repaint gating; egui version = nice-plug's | S | A, B, C, H (only one other heavy lane at a time) | No |
| **H — Minimal `tools/test-host`** on clack-host (1–2 d, heavy). Load a `.clap`, run a process loop, open/close the editor ×200, inject params, log PSS and timings as JSON | S (uses nice-plug's example plugin until A's bundle exists) | A, B, C, U1 | No |
| **U2 — Embed U1 in A's shell and measure** (2 d, heavy). The decision subset in REAPER and the test host; xruns with editor open vs closed (A/B); host checklist | A, U1, H | C's second listening session, P | **Yes:** checklist by hand: HiDPI, keyboard, drag, bridged mode (~1 h) |
| **P — Agent-loop probe** (optional, ½ d). `describe`/`check` over C's renderer; an agent attempts 5 text goals; log renders, tokens and failure causes | C | U2 | No |
| **V — vizia arm** (contingent, ~3 d) | U2 fails a threshold after tuning | — | As U2 |
| **Z — Close** (Task 7, ½ d). Thresholds; ADR-0001/0002 status; metric ADR draft; actual effort vs estimate per lane | A, B, C, U2 (P if run) | — | **Yes:** approve |

**Calendar (working days):**

- Day 0: O and T0.
- Day 1: S.
- Days 2–5: A and U1 (the two heavy lanes) alongside B, then C. H starts when U1 finishes.
- Days 5–6: the owner's host checks (A) and listening session 1 (C).
- Days 6–8: U2 and P; listening session 2.
- Day 9: Z.

The critical path is O → S → A → U2 → Z, about **9–10 working days**. If the owner frees about 8 GiB of RAM during work and moves the tree to ext4, up to four lanes can run at once and H can start on day 2.

## Plan check — concrete changes to the docs, with reasons

| Doc | Change | Reason |
|---|---|---|
| `docs/phase-0.md` (intro) | Replace "Do the tasks in order… waits" with: after the skeleton, run lanes in parallel per the schedule; each lane stops at its own acceptance; the owner reviews lane reports in one daily slot. Time box: about 2 weeks of calendar time, about 6–8 h of owner time | Q3: lanes depend only on the skeleton and the shell |
| `docs/phase-0.md` Task 1 | Rewrite as "Owner setup": native REAPER 7.82 tarball (user install); `carla` or `qtractor` as the second CLAP host; `pipewire-jack`, `hyperfine` via apt; clap-validator 0.4.1 binary; reboot; working tree and target on ext4; optional `preempt=full threadirqs` and RT limits. Acceptance is checked by `cargo xtask doctor` | Risks #1 and #2, measured gaps |
| `docs/README.md` (Developer environment), `docs/benchmarks.md` | "Second hosts: Bitwig and/or Ardour" → "Carla or Qtractor (CLAP); Bitwig trial only for the poly-mod check; Ardour only through VST3" | Ardour hosts no CLAP; Bitwig trial limits |
| `docs/phase-0.md` Task 3, `docs/adr/0001-plugin-shell.md` | Spike on the newest crates.io release, never a git dependency; record the release cadence and the single maintainer; budget an upgrade day a month; add checks for self-triggered audition notes and permanent IDs; move (d) to S1 | Risk #5; Codeberg outages; Recreate needs to audition notes by itself |
| `docs/phase-0.md` Task 4, `docs/benchmarks.md` (decision rules), `docs/adr/0002-ui-toolkit.md` | egui-only by default; vizia only if egui fails a threshold; remove the WebView arm; split into U1, H and U2; measure the decision subset; defer watts and 50-instance scaling to S1; expect keyboard-focus items to fail and log them | Risk #9; owner preference; saves 2–3 days |
| `docs/phase-0.md` Task 5 | Shrink; add the P1-like patch with a reverb stand-in, the seed → patch-hash test across platforms, and the `std` vs `libm` comparison; store the toolchain version next to golden hashes | Risk #7; Rust docs; design §7 budget realism |
| `docs/phase-0.md` Task 6, roadmap G0 | Triplet listening design (≥ 60 judgements, 10 repeats, two-thirds near threshold, ≤ 30 min sessions); gate "metric v1 matches the owner's consistent choices in ≥ 75% of near-threshold triplets; margins reported with CIs"; `metric v1` is provisional and recalibrated in GR1; **G0 metric results do not block P1** | Risk #3; statistics; Salimi 2026; ear fatigue |
| `docs/roadmap.md` (P1) | Split into **P1a — headless core** (params, patch and ops, engine, render, analysis descriptors, CLI v0, `enth random`/`compare`, plus a throwaway playable standalone Recreate prototype for the owner) and **P1b — plugin and editor** (shell, 72 host params, state, functional editor). Move the G1 agent gate and goldens to the end of P1a | Risk #4; fastest agent feedback loops are offline; tests the headline hypothesis about 4–6 weeks earlier |
| `docs/roadmap.md` (R1, GR1, S4) | Add the prerequisites "survey friends' OS/DAW" and "friend build": a standalone app through nice-plug's wrapper and/or a Windows VST3 build. Or redefine GR1 as owner-only, with friends in S1 | Risk #6; CLAP adoption; macOS Gatekeeper |
| `docs/design.md` §6 and §9 | Lowest difficulties use stepped (notched) values with audibly distinct steps and lead with structure scoring; the `sound` weight grows with difficulty. Target generation is parameter-space only (or the challenge code carries the accepted attempt index); "musical" priors are hand-written until enth has its own preset corpus | Q4 trainer evidence; risk #7; Q6.8 |
| `docs/design.md` §5 | Add: "DSP avoids `std` transcendental functions (use `libm` or our own approximations), or golden hashes are keyed by toolchain version" | Rust documents those functions as non-deterministic across platforms and versions |
| `docs/agent-cli.md`, roadmap G1 | Define the eval protocol: expectation lists given for the gate; 3 runs per goal, median reported; model name and date recorded; tokens and cost recorded; a blind human check that ≥ 7/10 final sounds match their text; agent-written expectations as a separate track; `agent-eval` runs nightly or on demand, never as a merge gate. Add lane P as an optional Phase 0 probe | Risk #8; thin evidence; non-determinism |
| `docs/naming.md` | Add: "Plugin IDs (the CLAP `id`, VST3 class IDs) are permanent and name-independent; choose them in Phase 0 before anyone saves a project." Add the 2026-10-19 crates.io and GitHub availability results; trademark search still open | Q6.6 |
| `docs/research-process.md` | Scale sub-researchers to milestone risk; cap `summary.md` at about 2 pages plus a decision table | Risk #10 |
| `docs/benchmarks.md` (procedure) | Measurement hygiene: rebooted machine, heavy apps closed, environment snapshot with every result (free RAM, swap, PRIME mode, preempt mode, PipeWire quantum, driver versions); xruns are attributed by an editor-open vs editor-closed A/B, not as an absolute zero | Risk #2, measured machine state |
| `docs/AGENTS.md` | Add: at most 2 concurrent heavy cargo builds; per-worktree `CARGO_TARGET_DIR` on ext4; write DSP from cited papers, never from memory of GPL synths; each lane writes only its own `docs/spikes/<lane>.md` | Q3 constraints; Q6.11 |

## Gaps — what I couldn't verify

- **Reddit** was blocked for my user agent, so first-hand player opinions on Syntorial, SoundGym and sound-match games come from reviews and a few forums only. I found no reviews of Synth Basics' sound match at all, and the page is undated.
- **Ardour 9.x and CLAP:** the "no CLAP" evidence is from 2024 maintainer statements, plus absence from clapdb and Ardour 9.0 coverage. I did not read `ardour.org/whatsnew` directly.
- **CLAP adoption:** the Antares (July 2026) and clapdb/Image-Line sources conflict on FL Studio and Studio One. I trusted the primary sources.
- **REAPER licence across platforms:** not stated on the purchase page. The 60-day full evaluation covers Phase 0 regardless.
- **Host behaviour for self-triggered notes** while the transport is stopped (REAPER, Carla, Qtractor): not verified; it is a check to add to Task 3.
- **METR's 2026 figures:** I used the post's summary (data "unreliable"), not its full tables.
- **Statistical sample sizes** are approximations (Fisher z; Steiger's test, applied to Spearman as if it were Pearson).
- **Build times on NTFS-3g vs ext4:** not measured here; sub-researcher 06 owns local timings.
- **Trademarks** (EUIPO, USPTO): not searched.
- **HexoSynth's history:** its repo is unreachable; no conclusion drawn.
- **Syntorial 2.0 "too subtle to hear" wording:** seen in search-result vendor copy (Plugin Boutique), not fetched. The fetched Best Service copy says "fewer frustrating roadblocks".
- **Apple Developer Program price:** taken from forum posts, not Apple's current page.
