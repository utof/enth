Goal: {brief}

A sound designer wrote this expectation list for the goal. The sound is accepted when `enth check` of your final
patch passes every item, with the performance below:

    {expect}

Performance (fixed; pass it to every `step` and `check`):

    --perf '{perf_short}'

Equivalent JSON form: {perf_json}

Start from the blank template (`enth new --template {template} --out {patch}`; `enth new --list` shows other
starting points) and shape the patch with `enth step`. Stay inside the {renders}-render budget. Items that pass by a
hair (`thin`) can flip with a small change, so leave room. Finish with the JSON answer, using the list above as `expect`.
