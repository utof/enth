//! **Owner: lane ONX** (freeze point F3s). Pre-declared by lane S0s (R3); empty until ONX writes it.
//!
//! Outputs to patches (docs/r3.md, "Our own scalar kernels"): normalised values to plain ones through `ParamSpec`
//! (patches store plain values in real units; the normalised domain exists only at the ML boundary), the categorical
//! heads' arg-max, and the whole onto the template's free set. The parameter order is tied to the catalogue and
//! generator versions in the model file's header.
