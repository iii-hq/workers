//! Integration test against a live iii engine.
//!
//! Runs only when env var `IIITEST_ENGINE_URL` is set; otherwise the test
//! returns early. Keeps cargo test green in CI without a running engine.

use std::sync::Arc;

use session_tree::{
    io::IIITrigger, store_iii_state::IiiStateSessionStore, SessionEntry, SessionMeta, SessionStore,
};

fn unique_session_id(prefix: &str) -> String {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{prefix}-{nonce}")
}

fn sample_meta(id: &str) -> SessionMeta {
    SessionMeta {
        session_id: id.to_string(),
        display_name: Some("integration".into()),
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
        custom_type: "integration".into(),
        content: serde_json::json!({"text": "hi"}),
        display: None,
        details: serde_json::Value::Null,
        timestamp: 1_500,
    }
}

#[tokio::test]
async fn round_trip_create_append_load_against_live_engine() -> anyhow::Result<()> {
    let Some(url) = std::env::var("IIITEST_ENGINE_URL").ok() else {
        eprintln!(
            "skipping integration_iii_state::round_trip_create_append_load_against_live_engine: \
             set IIITEST_ENGINE_URL to a running engine to enable"
        );
        return Ok(());
    };

    let iii = Arc::new(iii_sdk::register_worker(
        &url,
        iii_sdk::InitOptions::default(),
    ));
    let iii_for_store: Arc<dyn IIITrigger> = iii.clone();
    let store = IiiStateSessionStore::new(iii_for_store);

    let sid = unique_session_id("st-test");
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

    let meta = store
        .load_meta(&sid)
        .await
        .map_err(|e| anyhow::anyhow!("load_meta: {e}"))?;
    assert_eq!(meta.session_id, sid);

    Ok(())
}
