//! **Owner: lane LM** (the API and a basic body, freeze point F1l); **PLY** (the properties). Pre-declared by lane S0l (R2).
//!
//! The pure lesson lifecycle (docs/learn.md, "The lesson model"): `start` stashes your patch and loads the step's
//! start; while `Working`, patch predicates re-run after each delivered edit and sound checks run as a job once the
//! patch settles; a result for an older revision is refused; `hint` gives the authored hints first, then measured
//! ones; `show` applies the witness's next op; `next` and `back` move between steps; `end(Restore | Keep)`.
//! `&mut self`, with times from the shell: no clocks, threads or I/O. The session binds it to the live patch
//! (`enth-session`'s `learn`, lane SL). Empty until LM writes it.
