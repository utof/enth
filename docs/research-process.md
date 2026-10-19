# Research before every milestone

**No milestone starts with code. It starts with a deep-research session whose sub-reports are saved verbatim, then a summary you read, then doc updates — and only then tasks.** The point is to stay on the right track with current facts (versions, repo health, what people report today) instead of marinating in a model's training data.

## When

- Before every milestone in the [roadmap](roadmap.md): P0, P1, R1–R5, S1–S4.
- Before any decision that would be expensive to reverse (a new core dependency, a file format change, a new thread or process boundary).
- Small, reversible tasks inside a milestone don't need their own session.

## Where

```
_local/sessions/<YYYY-MM-DD>-<slug>/
├── brief.md                 # the questions, scope, constraints, milestone
├── 01-<topic>.md            # sub-researcher report, saved VERBATIM
├── 02-<topic>.md            # one file per sub-researcher
├── …
└── summary.md               # synthesis: answers, recommendations, decisions, open questions, docs changed
```

- `<YYYY-MM-DD>` is the session's start date; `<slug>` is short kebab-case, e.g. `2026-11-12-webview-implementation-and-benches`.
- Sub-reports are **never edited** after they're written. Corrections and disagreements go into `summary.md`.
- `_local/sessions/` is **committed** (decided 2026-10-19: the records are the project's memory). Everything else under `_local/` (for example `_local/reference/`, local UI reference screenshots) is never committed. Before the repo goes public, review committed briefs for personal details such as hardware and driver state.
- Example: `_local/sessions/2026-10-18-effects-reuse-and-plugin-hosting/`.

## How

1. **Brief.** Write `brief.md`: the milestone, its goal, the questions, and the constraints (MIT-compatible licences, real-time safety, determinism, wasm32 for core crates, Linux + REAPER first).
2. **Split into sub-researchers.** One per independent axis below that carries real risk, run in parallel (in Claude Code: one subagent each, with web search). Each saves its own report straight into the session folder. **Scale the number to the milestone's risk:** a small milestone needs 2–3 axes, not all of them. The last session produced reports of 40–55 KB each, and your review time is the limit, not the agents'.
3. **Summarise.** A separate pass reads all sub-reports and writes `summary.md`, with a *Corrections to sub-reports* section (where reports conflict with each other or with the docs) and an *Open checks* table, ending with concrete recommendations and the list of docs to change. **Keep it to about 2 pages plus a decision table** (area, decision, evidence, status). The details stay in the sub-reports.
4. **You read the summary.** Agree or redirect.
5. **Update docs.** ADRs, architecture, design, the milestone's task list. `summary.md` lists exactly which docs changed.
6. **Then build.**

### Axes every milestone session covers (where they apply)

| Axis | Questions |
| --- | --- |
| Code structure | Exact crates, modules and files for this milestone; key types and traits sketched; where tests live |
| Architecture | Decisions and alternatives; anything that touches layers, threads, formats or determinism; feeds ADRs |
| Libraries | Candidates with versions and repo health checked *today* (criteria below) |
| What people say | KVR, Gearspace, Reddit, Rust Audio community, GitHub issues: real-world reports, failures, praise. For DSP picks, forum opinion is thin: plan a blind, level-matched listening test before deciding |
| Papers and prior art | Research and existing products, where relevant |
| Risks and unknown unknowns | What could surprise us; what we haven't asked |

### Rules for sub-researchers

- Never put the owner's email, name or any personal detail in a request (User-Agent or other headers, URLs, payloads). APIs that ask for contact info (e.g. crates.io) get a generic User-Agent such as `enth-research/0.1`.
- Date every claim about versions, releases, activity or popularity; link the source.
- Verify licences from the actual LICENSE file or source headers. In mixed-licence repos, check the licence of the specific file, function or module.
- Mark each claim *measured*, *documented* or *anecdotal*.
- Don't state anything that changes over time from memory; look it up.
- Work in persistent scratch storage outside the repo (for example `~/code/enth-scratch/<session>/<NN>/`), never only in `/tmp`, and keep a running `notes.md` there. A crash or reboot loses `/tmp`; in the Phase 0 session two reboots did, and the notes are what let the work resume.
- Run anything that opens a window or loads a plugin editor under `xvfb-run` with Mesa software rendering, never on the real display ([AGENTS.md](../AGENTS.md)).
- Say plainly when evidence is missing. When the sandbox blocks something (the GitHub API, a rustup target, a site), list it under Gaps; the summary then names the build or health check (e.g. `cargo xtask wasm-check`, `gh api`) to run locally before deciding.

## Library choice criteria

A library must pass the two gates, then wins on the rest.

| Criterion | What "good" looks like |
| --- | --- |
| **Gate: licence** | MIT, Apache-2.0, ISC, BSD, Zlib, Unicode-3.0, CC0 / public domain (OFL/UFL for fonts). GPL family is reference-only |
| **Gate: real-time fit** (audio path only) | No allocation, locks or I/O in processing; deterministic; no hidden RNG or clocks |
| Activity | A commit and ideally a release in the last 6 months; issues get responses |
| Maintainers | More than one maintainer, or an organisation; a clear succession story |
| Adoption | Downloads, dependents and known real projects using it |
| Docs and examples | Enough that an agent can use it correctly without guessing |
| AI familiarity | Established, stable API that's well represented in public code (fewer hallucinated APIs); prefer it over a newer, niche alternative when quality is close |
| Platforms | Linux (X11) + Windows + macOS; wasm32 for anything in core crates |
| Exit cost | We wrap it behind our own trait or module so it can be replaced |

Record the comparison in the session summary and, for core dependencies, in an ADR.

## Kick-off prompt for a coding agent

> Run a research session for **<milestone>** following `docs/research-process.md`. Create `_local/sessions/<today>-<slug>/brief.md`, run one sub-researcher per relevant axis in parallel with web search, have each save its report verbatim in that folder, then write `summary.md` with recommendations and the docs to change. Stop and wait for me before changing docs or code.
