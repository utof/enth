//! **Owner: lane TP** (freeze point F1s, together with FE's intake types). Pre-declared by lane S0s (R3); empty until
//! TP writes it.
//!
//! `Candidate`: a patch, its distance, its **provenance** (heuristic, template, k-NN or model), and its trace of renders
//! and time; plus the types of the `create` entry point. This is the API the CLI (`enth.create/1`), the plugin, the
//! session's create state, the GR3 kit and R3b build on, so a judge reviews it at F1s; after F2s it changes only
//! additively (docs/phase-3.md, "Freezes"). Every result records its K starts as patches, the input features' hash and
//! the decoded audio's hash, so a run replays from its starts (docs/r3.md, "The determinism contract", point 4).
