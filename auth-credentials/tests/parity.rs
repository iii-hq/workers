//! Trait-parity tests: every behavior must hold for every CredentialStore impl.
//!
//! Parameterized tests run against InMemoryStore unconditionally and against
//! IiiStateCredentialStore when IIITEST_ENGINE_URL is set. Drift between the
//! two impls fails parity here before it can surprise a caller in production.

use std::sync::Arc;

use auth_credentials::{
    io::IIITrigger, store_iii_state::IiiStateCredentialStore, Credential, CredentialStore,
    InMemoryStore,
};

/// Construct a unique provider id per call so parallel test runs and reruns
/// against the same engine don't collide.
fn unique_provider(prefix: &str) -> String {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("test-{prefix}-{nonce}")
}

async fn parity_round_trip<S: CredentialStore>(store: &S) -> anyhow::Result<()> {
    let provider = unique_provider("rt");
    let cred = Credential::ApiKey {
        key: format!("sk-rt-{}", unique_provider("k")),
    };

    store.set(&provider, cred.clone()).await?;
    let got = store.get(&provider).await?;
    assert_eq!(got, Some(cred.clone()));

    store.clear(&provider).await?;
    assert_eq!(store.get(&provider).await?, None);
    Ok(())
}

async fn parity_set_overwrites_existing<S: CredentialStore>(store: &S) -> anyhow::Result<()> {
    let provider = unique_provider("ow");
    let cred1 = Credential::ApiKey {
        key: "first".into(),
    };
    let cred2 = Credential::ApiKey {
        key: "second".into(),
    };

    store.set(&provider, cred1).await?;
    store.set(&provider, cred2.clone()).await?;
    assert_eq!(store.get(&provider).await?, Some(cred2));

    store.clear(&provider).await?;
    Ok(())
}

async fn parity_clear_is_idempotent<S: CredentialStore>(store: &S) -> anyhow::Result<()> {
    let provider = unique_provider("ci");
    // Clearing a non-existent key should not error.
    store.clear(&provider).await?;
    store.clear(&provider).await?;
    assert_eq!(store.get(&provider).await?, None);
    Ok(())
}

async fn parity_list_includes_set_entries<S: CredentialStore>(store: &S) -> anyhow::Result<()> {
    let pa = unique_provider("la");
    let pb = unique_provider("lb");
    let ca = Credential::ApiKey { key: "ka".into() };
    let cb = Credential::ApiKey { key: "kb".into() };

    store.set(&pa, ca.clone()).await?;
    store.set(&pb, cb.clone()).await?;

    let listed = store.list().await?;
    // The store may contain entries from prior tests if running against a
    // shared engine; check that *both* of ours are present.
    assert!(
        listed.iter().any(|(p, c)| p == &pa && c == &ca),
        "list should contain ({pa}, ka); got {:?}",
        listed,
    );
    assert!(
        listed.iter().any(|(p, c)| p == &pb && c == &cb),
        "list should contain ({pb}, kb); got {:?}",
        listed,
    );

    store.clear(&pa).await?;
    store.clear(&pb).await?;
    Ok(())
}

async fn parity_get_missing_returns_none<S: CredentialStore>(store: &S) -> anyhow::Result<()> {
    let provider = unique_provider("missing");
    assert_eq!(store.get(&provider).await?, None);
    Ok(())
}

// === InMemoryStore (always runs) ===

#[tokio::test]
async fn in_memory_round_trip() -> anyhow::Result<()> {
    parity_round_trip(&InMemoryStore::new()).await
}

#[tokio::test]
async fn in_memory_set_overwrites_existing() -> anyhow::Result<()> {
    parity_set_overwrites_existing(&InMemoryStore::new()).await
}

#[tokio::test]
async fn in_memory_clear_is_idempotent() -> anyhow::Result<()> {
    parity_clear_is_idempotent(&InMemoryStore::new()).await
}

#[tokio::test]
async fn in_memory_list_includes_set_entries() -> anyhow::Result<()> {
    parity_list_includes_set_entries(&InMemoryStore::new()).await
}

#[tokio::test]
async fn in_memory_get_missing_returns_none() -> anyhow::Result<()> {
    parity_get_missing_returns_none(&InMemoryStore::new()).await
}

// === IiiStateCredentialStore (gated on IIITEST_ENGINE_URL) ===

fn live_store() -> Option<IiiStateCredentialStore> {
    let url = std::env::var("IIITEST_ENGINE_URL").ok()?;
    let iii = Arc::new(iii_sdk::register_worker(
        &url,
        iii_sdk::InitOptions::default(),
    ));
    let iii_for_store: Arc<dyn IIITrigger> = iii;
    Some(IiiStateCredentialStore::new(iii_for_store))
}

#[tokio::test]
async fn iii_state_round_trip() -> anyhow::Result<()> {
    let Some(store) = live_store() else {
        return Ok(());
    };
    parity_round_trip(&store).await
}

#[tokio::test]
async fn iii_state_set_overwrites_existing() -> anyhow::Result<()> {
    let Some(store) = live_store() else {
        return Ok(());
    };
    parity_set_overwrites_existing(&store).await
}

#[tokio::test]
async fn iii_state_clear_is_idempotent() -> anyhow::Result<()> {
    let Some(store) = live_store() else {
        return Ok(());
    };
    parity_clear_is_idempotent(&store).await
}

#[tokio::test]
async fn iii_state_list_includes_set_entries() -> anyhow::Result<()> {
    let Some(store) = live_store() else {
        return Ok(());
    };
    parity_list_includes_set_entries(&store).await
}

#[tokio::test]
async fn iii_state_get_missing_returns_none() -> anyhow::Result<()> {
    let Some(store) = live_store() else {
        return Ok(());
    };
    parity_get_missing_returns_none(&store).await
}
