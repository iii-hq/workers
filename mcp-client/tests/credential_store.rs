mod support;

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use mcp_client::credential_store::{VaultCredentialStore, encode};
use mcp_client::host::{FunctionDef, Handler, Host, Registration, SecretError, ServerEvent};
use mcp_client::translate::ServerSecrets;
use rmcp::transport::auth::{AuthError, CredentialStore, StoredCredentials};
use serde_json::{Value, json};
use support::FakeHost;

const RES: &str = "https://mcp.example/mcp";
const NAME: &str = "MCP_CLIENT_SRV_OAUTH";
const REF: &str = "secret://MCP_CLIENT_SRV_OAUTH";

fn creds(access: &str, refresh: &str) -> StoredCredentials {
    serde_json::from_value(json!({
        "client_id": "c1",
        "granted_scopes": ["read"],
        "token_received_at": 1_700_000_000u64,
        "issuer": "https://as.example",
        "token_response": {
            "access_token": access,
            "token_type": "Bearer",
            "expires_in": 3600,
            "refresh_token": refresh
        }
    }))
    .unwrap()
}

fn store(host: Arc<dyn Host>, rejected: &Arc<AtomicUsize>) -> VaultCredentialStore {
    let rejected = rejected.clone();
    VaultCredentialStore::new(
        host,
        "srv",
        RES,
        Arc::new(move || {
            rejected.fetch_add(1, Ordering::SeqCst);
        }),
    )
}

fn stored(host: &FakeHost) -> Value {
    serde_json::from_str(&host.vault(NAME).expect("no stored session")).unwrap()
}

fn json_of(credentials: &StoredCredentials) -> Value {
    serde_json::to_value(credentials).unwrap()
}

fn is_store_error<T>(result: &Result<T, AuthError>) -> bool {
    matches!(result, Err(AuthError::CredentialStoreError(_)))
}

/// Delegates to the `FakeHost`, except that its first `resolve` of
/// `reference` returns the current text and then stores `intruder`.
struct RacingHost {
    host: Arc<FakeHost>,
    reference: String,
    intruder: String,
    raced: AtomicBool,
}

impl RacingHost {
    fn new(host: Arc<FakeHost>, reference: &str, intruder: &str) -> Arc<RacingHost> {
        Arc::new(RacingHost {
            host,
            reference: reference.into(),
            intruder: intruder.into(),
            raced: AtomicBool::new(false),
        })
    }
}

#[async_trait::async_trait]
impl Host for RacingHost {
    fn register(&self, def: &FunctionDef, handler: Handler) -> Box<dyn Registration> {
        self.host.register(def, handler)
    }

    async fn registered(&self) -> HashSet<String> {
        self.host.registered().await
    }

    async fn resolve(&self, reference: &str) -> Result<String, SecretError> {
        let current = self.host.resolve(reference).await;
        if reference == self.reference && !self.raced.swap(true, Ordering::SeqCst) {
            self.host.set_secret(&self.reference, &self.intruder);
        }
        current
    }

    async fn compare_and_set(
        &self,
        name: &str,
        expected: Option<&str>,
        value: &str,
    ) -> Result<bool, SecretError> {
        self.host.compare_and_set(name, expected, value).await
    }

    fn emit(&self, event: ServerEvent) {
        self.host.emit(event);
    }

    async fn http_port(&self) -> Option<u16> {
        self.host.http_port().await
    }
}

#[tokio::test]
async fn save_creates_the_secret_and_load_returns_it() {
    let host = FakeHost::new();
    let rejected = Arc::new(AtomicUsize::new(0));
    let s = store(host.clone(), &rejected);
    s.save(creds("at-1", "rt-1")).await.unwrap();
    let saved = stored(&host);
    assert_eq!(saved["version"], 1);
    assert_eq!(saved["resource"], RES);
    assert_eq!(saved["credentials"]["client_id"], "c1");
    let loaded = s.load().await.unwrap().expect("no session loaded");
    assert_eq!(json_of(&loaded), json_of(&creds("at-1", "rt-1")));
}

#[tokio::test]
async fn load_has_no_cache() {
    let host = FakeHost::new();
    let rejected = Arc::new(AtomicUsize::new(0));
    let s = store(host.clone(), &rejected);
    s.save(creds("at-1", "rt-1")).await.unwrap();
    let before = host.resolve_count(REF);
    s.load().await.unwrap();
    s.load().await.unwrap();
    assert_eq!(host.resolve_count(REF), before + 2);
    host.set_secret(REF, &encode(RES, None));
    assert!(s.load().await.unwrap().is_none());
}

#[tokio::test]
async fn a_session_stored_for_another_url_is_not_loaded() {
    let host = FakeHost::new();
    let rejected = Arc::new(AtomicUsize::new(0));
    store(host.clone(), &rejected)
        .save(creds("at-1", "rt-1"))
        .await
        .unwrap();
    let other = VaultCredentialStore::new(
        host.clone(),
        "srv",
        "https://other.example/mcp",
        Arc::new(|| {}),
    );
    assert!(other.load().await.unwrap().is_none());
}

#[tokio::test]
async fn a_missing_secret_is_no_session_and_secrets_down_is_a_store_error() {
    let host = FakeHost::new();
    let rejected = Arc::new(AtomicUsize::new(0));
    let s = store(host.clone(), &rejected);
    assert!(s.load().await.unwrap().is_none());
    host.set_unreachable(true);
    assert!(is_store_error(&s.load().await));
    assert!(is_store_error(&s.save(creds("at-1", "rt-1")).await));
}

#[tokio::test]
async fn save_does_not_overwrite_on_a_lost_swap() {
    let host = FakeHost::new();
    let rejected = Arc::new(AtomicUsize::new(0));
    let intruder = encode(
        RES,
        Some(&json!({"client_id": "someone-else", "token_response": null})),
    );
    host.set_secret(REF, &encode(RES, None));
    let s = store(RacingHost::new(host.clone(), REF, &intruder), &rejected);
    assert!(is_store_error(&s.save(creds("at-1", "rt-1")).await));
    assert_eq!(host.vault(NAME), Some(intruder));
}

#[tokio::test]
async fn clear_writes_null_credentials_and_keeps_the_envelope() {
    let host = FakeHost::new();
    let rejected = Arc::new(AtomicUsize::new(0));
    let s = store(host.clone(), &rejected);
    s.save(creds("at-1", "rt-1")).await.unwrap();
    s.clear().await.unwrap();
    assert_eq!(
        stored(&host),
        json!({"version": 1, "resource": RES, "credentials": null})
    );
    assert!(s.load().await.unwrap().is_none());
}

#[tokio::test]
async fn clear_with_nothing_stored_writes_nothing() {
    let host = FakeHost::new();
    let rejected = Arc::new(AtomicUsize::new(0));
    store(host.clone(), &rejected).clear().await.unwrap();
    assert_eq!(host.vault(NAME), None);
}

#[tokio::test]
async fn a_rejected_refresh_token_clears_only_the_session_it_belongs_to() {
    let host = FakeHost::new();
    let rejected = Arc::new(AtomicUsize::new(0));
    let s = store(host.clone(), &rejected);
    s.save(creds("at-2", "rt-2")).await.unwrap();
    s.on_refresh_token_rejected(&creds("at-1", "rt-1"))
        .await
        .unwrap();
    assert_eq!(stored(&host)["credentials"]["client_id"], "c1");
    assert_eq!(rejected.load(Ordering::SeqCst), 0);
    s.on_refresh_token_rejected(&creds("at-2", "rt-2"))
        .await
        .unwrap();
    assert_eq!(stored(&host)["credentials"], Value::Null);
    assert_eq!(rejected.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn the_refresh_guard_is_shared_by_clones_and_released_on_drop() {
    let host = FakeHost::new();
    let rejected = Arc::new(AtomicUsize::new(0));
    let s = store(host.clone(), &rejected);
    let guard = s.acquire_refresh_guard().await.unwrap();
    let other = s.clone();
    let held = tokio::time::timeout(Duration::from_millis(50), other.acquire_refresh_guard()).await;
    assert!(held.is_err(), "a clone took the guard while it was held");
    drop(guard);
    let freed = tokio::time::timeout(Duration::from_secs(1), other.acquire_refresh_guard()).await;
    assert!(freed.is_ok(), "the guard was not released on drop");
}

#[tokio::test]
async fn sign_out_clears_the_session_without_calling_on_rejected() {
    let host = FakeHost::new();
    let rejected = Arc::new(AtomicUsize::new(0));
    let s = store(host.clone(), &rejected);
    s.save(creds("at-1", "rt-1")).await.unwrap();
    s.sign_out().await.unwrap();
    assert_eq!(stored(&host)["credentials"], Value::Null);
    assert_eq!(rejected.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn sign_out_waits_for_a_refresh_in_flight() {
    let host = FakeHost::new();
    let rejected = Arc::new(AtomicUsize::new(0));
    let s = store(host.clone(), &rejected);
    let guard = s.acquire_refresh_guard().await.unwrap();
    let mut signing_out = tokio::spawn({
        let s = s.clone();
        async move { s.sign_out().await }
    });
    // The refresh holding the guard saves its new tokens.
    s.save(creds("at-2", "rt-2")).await.unwrap();
    let early = tokio::time::timeout(Duration::from_millis(50), &mut signing_out).await;
    assert!(
        early.is_err(),
        "sign-out ran while a refresh held the guard"
    );
    drop(guard);
    signing_out.await.unwrap().unwrap();
    assert_eq!(stored(&host)["credentials"], Value::Null);
}

#[tokio::test]
async fn loaded_and_saved_tokens_join_the_server_secrets() {
    let host = FakeHost::new();
    let rejected = Arc::new(AtomicUsize::new(0));
    let saved = ServerSecrets::new();
    store(host.clone(), &rejected)
        .with_secrets(saved.clone())
        .save(creds("at-1", "rt-1"))
        .await
        .unwrap();
    let mut values = saved.values();
    values.sort();
    assert_eq!(values, ["at-1", "rt-1"]);

    let loaded = ServerSecrets::new();
    let s = store(host.clone(), &rejected).with_secrets(loaded.clone());
    s.load().await.unwrap();
    let mut values = loaded.values();
    values.sort();
    assert_eq!(values, ["at-1", "rt-1"]);
    assert!(loaded.resolved().is_empty());
}

#[tokio::test]
async fn an_id_token_rmcp_keeps_joins_the_server_secrets() {
    let mut credentials = json_of(&creds("at-1", "rt-1"));
    credentials["token_response"]["id_token"] = json!("idt-1");
    let credentials: StoredCredentials = serde_json::from_value(credentials).unwrap();
    assert_eq!(
        json_of(&credentials)["token_response"]["id_token"],
        "idt-1",
        "rmcp keeps the extra token field"
    );
    let saved = ServerSecrets::new();
    let rejected = Arc::new(AtomicUsize::new(0));
    store(FakeHost::new(), &rejected)
        .with_secrets(saved.clone())
        .save(credentials)
        .await
        .unwrap();
    assert!(saved.values().contains(&"idt-1".to_string()));
}
