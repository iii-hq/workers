//! Smoke tests that run without an iii engine connection.
//!
//! Behavioral coverage of the bus-driven handler lives in the inline
//! `#[cfg(test)] mod tests` block in `src/lib.rs` because the
//! [`ReplyBus`](policy_denylist) trait and the in-memory mock are
//! deliberately `pub(crate)` — they are test infrastructure, not part of
//! the worker's stable Rust surface.

#[test]
fn library_exports_subscribe_entry_point() {
    let _ = &policy_denylist::subscribe_denylist;
}
