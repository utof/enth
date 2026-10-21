You are working through one lesson of a synthesizer course with a command-line tool named `enth`. You cannot hear:
`enth` renders a patch and measures it, and you work from the lesson's text and those numbers. Your only tool is a
shell that runs `enth` commands, one at a time (no pipes, no other programs). Run `enth guide` first; it explains the
commands, and `enth lesson --help` the lesson ones.

The lesson is in the pack in the `{pack}` directory of your working directory. `enth lesson show {pack} --lesson
{lesson}` prints its steps, in order: each step's text, the controls it lets you edit or locks, its check, and how
many authored hints it has. Read it once, then do the steps in order. A step is one of:
- `read`, `listen`: nothing to hand in. A listen step plays sounds you cannot hear; `enth lesson start` can write them
  as files for `enth describe` (below).
- `do`: make a patch that passes the step's check. `enth lesson start {pack} --lesson {lesson} --step ID --out
  FILE.patch.json` writes the patch the step opens with (a step with no start of its own carries on from the patch
  you ended the step before with); change it with `enth apply`, and test it with `enth lesson check {pack} --lesson
  {lesson} --step ID FILE.patch.json --from START.patch.json` (exit 0 passes). `--from` is the patch the step opened
  with: a check that a control "was turned" reads how far it moved from there, so keep an untouched copy of the start
  (write it a second time under another name).
- `recreate`: a round. `enth lesson start ... --out FILE.patch.json --challenge-out ROUND.json` writes the round's
  start and the round itself; a hidden target sound is drawn. Edit your patch until `enth compare FILE.patch.json
  --challenge ROUND.json` passes (exit 0). You cannot hear the target, so you search: `compare` says how close you are
  and which aspect differs.
- `choose`: an ear question. `enth lesson start ... --examples-out DIR` writes each option as a patch and a performance
  file; measure them with `enth describe DIR/N-LABEL.patch.json --perf DIR/N-LABEL.perf.json` and answer with the
  label of the option that fits the step's text.

Rules of this exercise:
- A budget of {units} units applies. Costs: `enth lesson check`, `compare`, `describe` or `check` of a patch, `render`,
  `step`, and a measured hint (`enth lesson hint` given a patch file) cost 1; `suggest` 2; `enth lesson closest` and
  `match` 10. Everything else is free: `lesson show`, `list` and `start`, an authored hint (`lesson hint --n 1` up to the
  step's number of authored hints, with no patch), `apply`, `get`, `diff`, `params` and `guide`. A call the tool rejects
  before rendering (a usage or validation error) is not charged. After the budget the tool refuses what costs, with
  `budget_exhausted`; then stop and answer with what you have.
- Hints are part of the lesson. `enth lesson hint` and `closest` are the lesson's own help: use them when you are
  stuck, and know that they are counted as assistance (the result records which steps they helped with).
- Stay inside a step's controls: a patch that changes a control the step locks, or one its `editable` list leaves out,
  does not count, even when `lesson check` passes it.
- `--reveal` (a lesson's answers, a round's target) is refused. So is a path outside the working directory, and the
  pack's own files except through `enth lesson`.
- When you are done, answer with a JSON object only: `{"steps": [{"step": "<id>", "patch": "<path of the patch for
  that step>"}, {"step": "<id>", "choice": "<label of your answer>"}], "explanation": "<one sentence>"}`. List every
  `do` and `recreate` step with its final patch file and every `choose` step with its label. A step left out counts as
  not done.
