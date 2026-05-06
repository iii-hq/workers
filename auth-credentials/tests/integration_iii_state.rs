//! Integration test against a live iii engine.
//!
//! Runs only when env var `IIITEST_ENGINE_URL` is set; otherwise the test
//! returns early. This keeps `cargo test` green in CI without a running
//! engine, while still exercising the wire path when the env is provided.

use std::sync::Arc;

use auth_credentials::{
    io::IIITrigger, store_iii_state::IiiStateCredentialStore, Credential, CredentialStore,
};

#[tokio::test]
async fn round_trip_set_get_clear_against_live_engine() -> anyhow::Result<()> {
    let Some(url) = std::env::var("IIITEST_ENGINE_URL").ok() else {
        eprintln!(
            "skipping integration_iii_state::round_trip_set_get_clear_against_live_engine: \
             set IIITEST_ENGINE_URL to a running engine to enable"
        );
        return Ok(());
    };

    // Construct an Arc<III> via register_worker (matches main.rs).
    let iii = Arc::new(iii_sdk::register_worker(
        &url,
        iii_sdk::InitOptions::default(),
    ));
    let iii_for_store: Arc<dyn IIITrigger> = iii.clone();
    let store = IiiStateCredentialStore::new(iii_for_store);

    // Use a unique provider id per run so concurrent test runs don't collide.
    // Nanos-since-epoch keeps this dep-free; uuid is not in the workspace.
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let provider = format!("test-provider-{nonce}");
    let cred = Credential::ApiKey {
        key: format!("sk-test-{nonce}"),
    };

    // Set, then get, asserting persistence.
    store.set(&provider, cred.clone()).await?;
    let got = store.get(&provider).await?;
    assert_eq!(got, Some(cred));

    // Clear, then get returns None.
    store.clear(&provider).await?;
    assert_eq!(store.get(&provider).await?, None);

    Ok(())
}
