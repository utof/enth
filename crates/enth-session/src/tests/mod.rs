//! Unit tests that need the crate's internals: an active session built by hand ([`kit`]) so undo, redo, delivery,
//! the fold against the plan's value and the host-load policy run against a real engine without lane SK's
//! `activate`. The public-API tests are in `tests/`.

mod deliver;
mod fold;
mod history;
mod host_load;
mod kit;
