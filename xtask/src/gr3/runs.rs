//! **Owner: lane EVK** (the append), **`EVb`** (the test runs). Pre-declared by lane S0s (R3); empty until EVK writes it.
//!
//! `content/gr3/runs.jsonl`: every test run, one JSON line each (the build, the date, the load), **appended and never
//! dropped or rewritten** (docs/r3.md, "Hygiene"). A run records the renders it used, so the render-budget twin
//! reproduces it on any machine, and its wall-clock load, so a timed run is judged as what it was. The file itself is
//! created by EVK on the first run; lane S0s did not create it.
