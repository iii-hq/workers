//! Smoke tests that run without an iii engine connection.

#[test]
fn in_memory_store_constructs() {
    let _store = session_tree::InMemoryStore::default();
}
