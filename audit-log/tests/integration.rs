//! Smoke tests that run without an iii engine connection.
//!
//! Behavioral coverage of the bus-driven handler lives in the inline
//! `#[cfg(test)] mod tests` block in `src/lib.rs` because the
//! [`ReplyBus`](audit_log) trait and the in-memory mock are
//! deliberately `pub(crate)` — they are test infrastructure, not part of
//! the worker's stable Rust surface.

#[test]
fn library_exports_subscribe_entry_point() {
    let _ = &audit_log::subscribe_audit_log;
}
