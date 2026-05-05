//! Trait-parity tests: every behavior must hold for every SessionStore impl.
//!
//! In-memory tests run unconditionally; iii-state tests gate on
//! IIITEST_ENGINE_URL. Drift between impls fails parity here before it can
//! surprise a caller in production.

use std::sync::Arc;

use session_tree::{
    io::IIITrigger,
    store_iii_state::IiiStateSessionStore,
    InMemoryStore, SessionEntry, SessionError, SessionMeta, SessionStore,
};

fn unique_session_id(prefix: &str) -> String {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("test-{prefix}-{nonce}")
}

fn sample_meta(id: &str) -> SessionMeta {
    SessionMeta {
        session_id: id.to_string(),
        display_name: Some("parity".into()),
        created_at: 1_000,
        updated_at: 1_000,
        cwd: None,
        branch_count: 1,
    }
}

fn sample_entry(eid: &str) -> SessionEntry {
    SessionEntry::CustomMessage {
        id: eid.to_string(),
        parent_id: None,
        custom_type: "parity".into(),
        content: serde_json::json!({"text": "hi"}),
        display: None,
        details: serde_json::Value::Null,
        timestamp: 1_500,
    }
}

async fn parity_create_then_load_meta<S: SessionStore>(store: &S) -> anyhow::Result<()> {
    let sid = unique_session_id("create");
    store
        .create(sample_meta(&sid))
        .await
        .map_err(|e| anyhow::anyhow!("create: {e}"))?;
    let loaded = store
        .load_meta(&sid)
        .await
        .map_err(|e| anyhow::anyhow!("load_meta: {e}"))?;
    assert_eq!(loaded.session_id, sid);
    Ok(())
}

async fn parity_append_then_load_entries<S: SessionStore>(store: &S) -> anyhow::Result<()> {
    let sid = unique_session_id("append");
    store
        .create(sample_meta(&sid))
        .await
        .map_err(|e| anyhow::anyhow!("create: {e}"))?;
    store
        .append(&sid, sample_entry("01"))
        .await
        .map_err(|e| anyhow::anyhow!("append 01: {e}"))?;
    store
        .append(&sid, sample_entry("02"))
        .await
        .map_err(|e| anyhow::anyhow!("append 02: {e}"))?;
    let entries = store
        .load_entries(&sid)
        .await
        .map_err(|e| anyhow::anyhow!("load_entries: {e}"))?;
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].id(), "01");
    assert_eq!(entries[1].id(), "02");
    Ok(())
}

async fn parity_load_meta_returns_not_found<S: SessionStore>(store: &S) -> anyhow::Result<()> {
    let result = store.load_meta("definitely-missing-session-id-xyz").await;
    assert!(
        matches!(result, Err(SessionError::NotFound(_))),
        "load_meta on missing session should return NotFound; got {:?}",
        result,
    );
    Ok(())
}

async fn parity_list_includes_created_sessions<S: SessionStore>(store: &S) -> anyhow::Result<()> {
    let sid_a = unique_session_id("list-a");
    let sid_b = unique_session_id("list-b");
    store
        .create(sample_meta(&sid_a))
        .await
        .map_err(|e| anyhow::anyhow!("create a: {e}"))?;
    store
        .create(sample_meta(&sid_b))
        .await
        .map_err(|e| anyhow::anyhow!("create b: {e}"))?;
    let listed = store
        .list()
        .await
        .map_err(|e| anyhow::anyhow!("list: {e}"))?;
    // The store may contain entries from prior tests if running against a
    // shared engine; assert both of ours are present.
    assert!(
        listed.iter().any(|m| m.session_id == sid_a),
        "list should contain {sid_a}; got {:?}",
        listed.iter().map(|m| &m.session_id).collect::<Vec<_>>(),
    );
    assert!(
        listed.iter().any(|m| m.session_id == sid_b),
        "list should contain {sid_b}",
    );
    Ok(())
}

// === InMemoryStore (always runs) ===

#[tokio::test]
async fn in_memory_create_then_load_meta() -> anyhow::Result<()> {
    parity_create_then_load_meta(&InMemoryStore::new()).await
}

#[tokio::test]
async fn in_memory_append_then_load_entries() -> anyhow::Result<()> {
    parity_append_then_load_entries(&InMemoryStore::new()).await
}

#[tokio::test]
async fn in_memory_load_meta_returns_not_found() -> anyhow::Result<()> {
    parity_load_meta_returns_not_found(&InMemoryStore::new()).await
}

#[tokio::test]
async fn in_memory_list_includes_created_sessions() -> anyhow::Result<()> {
    parity_list_includes_created_sessions(&InMemoryStore::new()).await
}

// === IiiStateSessionStore (gated on IIITEST_ENGINE_URL) ===

fn live_store() -> Option<IiiStateSessionStore> {
    let url = std::env::var("IIITEST_ENGINE_URL").ok()?;
    let iii = Arc::new(iii_sdk::register_worker(&url, iii_sdk::InitOptions::default()));
    let iii_for_store: Arc<dyn IIITrigger> = iii;
    Some(IiiStateSessionStore::new(iii_for_store))
}

#[tokio::test]
async fn iii_state_create_then_load_meta() -> anyhow::Result<()> {
    let Some(store) = live_store() else { return Ok(()); };
    parity_create_then_load_meta(&store).await
}

#[tokio::test]
async fn iii_state_append_then_load_entries() -> anyhow::Result<()> {
    let Some(store) = live_store() else { return Ok(()); };
    parity_append_then_load_entries(&store).await
}

#[tokio::test]
async fn iii_state_load_meta_returns_not_found() -> anyhow::Result<()> {
    let Some(store) = live_store() else { return Ok(()); };
    parity_load_meta_returns_not_found(&store).await
}

#[tokio::test]
async fn iii_state_list_includes_created_sessions() -> anyhow::Result<()> {
    let Some(store) = live_store() else { return Ok(()); };
    parity_list_includes_created_sessions(&store).await
}
